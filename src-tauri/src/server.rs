use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use percent_encoding::percent_decode_str;
use serde_json::Value;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::settings::{ConfigSource, LoadedUi};

pub struct Session {
    pub ui_dir: Option<PathBuf>,
    pub ui_source: String,
    pub loaded: Option<LoadedUi>,
    pub status: String,
}

pub struct ServerHandle {
    pub url: String,
    server: Arc<Server>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn serve(shared: Arc<Mutex<Session>>, port: u16) -> Result<ServerHandle, String> {
    let server =
        Arc::new(Server::http(format!("127.0.0.1:{port}")).map_err(|err| err.to_string())?);
    let addr = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr,
        other => return Err(format!("Unexpected listen address: {other}")),
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    let thread_server = Arc::clone(&server);
    let thread = std::thread::spawn(move || {
        loop {
            if thread_shutdown.load(Ordering::SeqCst) {
                break;
            }
            let request = match thread_server.recv() {
                Ok(request) => request,
                Err(_) => break,
            };
            if thread_shutdown.load(Ordering::SeqCst) {
                let _ = request.respond(Response::empty(StatusCode(204)));
                break;
            }
            if let Err(err) = dispatch(request, &shared) {
                crate::journal::write("local page", &err.to_string());
            }
        }
    });
    Ok(ServerHandle {
        url: format!("http://{addr}"),
        server,
        shutdown,
        thread: Some(thread),
    })
}

pub fn serve_local(shared: Arc<Mutex<Session>>) -> Result<ServerHandle, String> {
    match serve(Arc::clone(&shared), 47_321) {
        Ok(handle) => Ok(handle),
        Err(_) => serve(shared, 0),
    }
}

fn dispatch(request: Request, shared: &Arc<Mutex<Session>>) -> io::Result<()> {
    let method = request.method().clone();
    let url = request.url().to_string();
    match method {
        Method::Get | Method::Head => {}
        _ => {
            return request.respond(plain(405, "Method not allowed"));
        }
    }
    let session = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let built = build_response(&session, &url);
    drop(session);
    let mut response = Response::from_data(if method == Method::Head {
        Vec::new()
    } else {
        built.body
    })
    .with_status_code(StatusCode(built.status));
    response = response.with_header(header("Content-Type", &built.content_type)?);
    response = response.with_header(header("Cache-Control", &built.cache_control)?);
    response = response.with_header(header("X-Content-Type-Options", "nosniff")?);
    request.respond(response)
}

struct Built {
    status: u16,
    content_type: String,
    cache_control: String,
    body: Vec<u8>,
}

fn build_response(session: &Session, url: &str) -> Built {
    let Some(relative) = safe_relative(url) else {
        return plain_built(400, "Bad path");
    };
    let (Some(ui_dir), Some(loaded)) = (session.ui_dir.as_ref(), session.loaded.as_ref()) else {
        return plain_built(503, "Choose a web app bundle in Buho.");
    };
    if relative == loaded.manifest.config_file && loaded.source == ConfigSource::Persisted {
        let body = serde_json::to_vec_pretty(&Value::Object(loaded.stored.clone()))
            .unwrap_or_else(|_| b"{}".to_vec());
        return Built {
            status: 200,
            content_type: "application/json".into(),
            cache_control: "no-store".into(),
            body,
        };
    }

    let root = match ui_dir.canonicalize() {
        Ok(root) => root,
        Err(_) => return plain_built(503, "Web app folder is not available"),
    };
    let candidate = root.join(&relative);
    if let Some(file) = file_inside(&root, &candidate) {
        return file_built(&file);
    }
    if loaded.manifest.spa && !has_extension(&relative) {
        if let Some(index) = file_inside(&root, &root.join("index.html")) {
            return file_built(&index);
        }
    }
    plain_built(404, "Not found")
}

fn file_inside(root: &Path, candidate: &Path) -> Option<PathBuf> {
    if !candidate.is_file() {
        return None;
    }
    let canon = candidate.canonicalize().ok()?;
    if canon.starts_with(root) {
        Some(canon)
    } else {
        None
    }
}

fn file_built(path: &Path) -> Built {
    match fs::read(path) {
        Ok(body) => Built {
            status: 200,
            content_type: content_type(path).into(),
            cache_control: "no-cache".into(),
            body,
        },
        Err(_) => plain_built(500, "Could not read file"),
    }
}

fn plain(status: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let built = plain_built(status, message);
    let mut response = Response::from_data(built.body).with_status_code(StatusCode(status));
    if let Ok(content_type) = header("Content-Type", &built.content_type) {
        response = response.with_header(content_type);
    }
    response
}

fn plain_built(status: u16, message: &str) -> Built {
    Built {
        status,
        content_type: "text/plain; charset=utf-8".into(),
        cache_control: "no-store".into(),
        body: message.as_bytes().to_vec(),
    }
}

fn safe_relative(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or("");
    let path = path.split('#').next().unwrap_or("");
    let decoded = percent_decode_str(path).decode_utf8().ok()?;
    let trimmed = decoded.trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() {
        return Some("index.html".into());
    }
    let mut parts = Vec::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.contains('\\') || part.contains('\0') {
                    return None;
                }
                parts.push(part.into_owned());
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    if parts.is_empty() {
        Some("index.html".into())
    } else {
        Some(parts.join("/"))
    }
}

fn has_extension(relative: &str) -> bool {
    Path::new(relative).extension().is_some()
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "wasm" => "application/wasm",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

fn header(name: &str, value: &str) -> io::Result<Header> {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid header"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{LoadedUi, load_ui, save_values};
    use serde_json::{Map, Value};
    use std::io::{ErrorKind, Read, Write};
    use std::net::{Shutdown, TcpStream};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::time::Duration;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "app-shell-http-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    struct Raw {
        status: u16,
        headers: String,
        body: String,
    }

    fn get(url: &str, path: &str) -> Raw {
        let host = url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(host).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let _ = stream.shutdown(Shutdown::Write);
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            if http_message_complete(&bytes) {
                break;
            }
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(size) => bytes.extend_from_slice(&chunk[..size]),
                Err(err)
                    if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock =>
                {
                    break;
                }
                Err(err) => panic!("reading {path} failed: {err}"),
            }
        }
        let text = String::from_utf8_lossy(&bytes);
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_ref(), ""));
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        Raw {
            status,
            headers: head.to_string(),
            body: body.to_string(),
        }
    }

    fn http_message_complete(bytes: &[u8]) -> bool {
        let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let head = String::from_utf8_lossy(&bytes[..split]);
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        bytes.len() >= split + 4 + length
    }

    fn session(loaded: LoadedUi, ui_dir: PathBuf) -> Session {
        Session {
            ui_dir: Some(ui_dir),
            ui_source: "saved".into(),
            loaded: Some(loaded),
            status: String::new(),
        }
    }

    #[test]
    fn serves_the_app_overlays_saved_config_and_blocks_escape() {
        let scratch = Scratch::new();
        let ui = scratch.0.join("ui");
        let data = scratch.0.join("data");
        write(&ui.join("index.html"), "APP_HOME");
        write(&ui.join("assets").join("app.js"), "console.log(1)");
        write(
            &ui.join("shell.manifest.json"),
            r#"{"id":"demo","name":"Demo","spa":true,"fields":[
              {"key":"apiUrl","label":"API URL","type":"url","required":true}
            ]}"#,
        );
        write(
            &ui.join("config.json"),
            r#"{"apiUrl":"http://disk.example"}"#,
        );
        let mut loaded = load_ui(&data, &ui).unwrap();
        let shared = Arc::new(Mutex::new(session(loaded.clone(), ui.clone())));
        let server = serve(Arc::clone(&shared), 0).unwrap();

        let home = get(&server.url, "/");
        assert_eq!(home.status, 200);
        assert_eq!(home.body, "APP_HOME");

        let disk = get(&server.url, "/config.json");
        assert!(disk.body.contains("disk.example"));

        let route = get(&server.url, "/records/12");
        assert_eq!(route.status, 200);
        assert_eq!(route.body, "APP_HOME");

        let missing = get(&server.url, "/assets/missing.js");
        assert_eq!(missing.status, 404);

        let escaped = get(&server.url, "/%2e%2e/%2e%2e/Cargo.toml");
        assert_ne!(escaped.status, 200);

        let mut values = Map::new();
        values.insert(
            "apiUrl".into(),
            Value::String("http://127.0.0.1:3002".into()),
        );
        save_values(&data, &mut loaded, &Value::Object(values)).unwrap();
        shared.lock().unwrap().loaded = Some(loaded);
        let saved = get(&server.url, "/config.json");
        assert_eq!(saved.status, 200);
        assert!(saved.headers.to_ascii_lowercase().contains("no-store"));
        assert!(saved.body.contains("127.0.0.1:3002"));
        assert!(!saved.body.contains("disk.example"));

        let script = get(&server.url, "/assets/app.js");
        assert_eq!(script.status, 200);
        assert!(script.headers.contains("javascript"));
    }
}
