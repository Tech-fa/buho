use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, Runtime};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

const BRIDGE: &str = include_str!("bridge.js");

/// Webview that shows the page the user opened. The agent tools drive this webview.
pub const PAGE_WEBVIEW: &str = "app";
pub const LEAVE_CONFIRMATION_TO_THE_USER: &str = "After you change the page, stop. Do not call submit_form, and do not click a confirmation button. The user approves the change by clicking that button.";

fn base_instructions() -> String {
    format!(
        "These tools control the app window open in Buho. Call list_pages, then navigate, then describe_page, then set_fields. Paths that contain :name need a real id from the open page. The tools change the window the user is looking at. {LEAVE_CONFIRMATION_TO_THE_USER}"
    )
}

pub struct BridgeTarget {
    pub setup_url: Mutex<String>,
    pub script: Mutex<String>,
}

pub fn bridge_script() -> &'static str {
    BRIDGE
}

pub fn bridge_plugin<R: Runtime>(target: Arc<BridgeTarget>) -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("app-control")
        .on_page_load(move |webview, payload| {
            use tauri::webview::PageLoadEvent;
            if payload.event() != PageLoadEvent::Finished || webview.label() != PAGE_WEBVIEW {
                return;
            }
            let setup = target
                .setup_url
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let Ok(setup_url) = Url::parse(&setup) else {
                return;
            };
            if is_shell_page(payload.url(), &setup_url) {
                return;
            }
            let script = target
                .script
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if !script.is_empty() {
                let _ = webview.eval(script);
            }
        })
        .build()
}

pub fn is_shell_page(page: &Url, setup: &Url) -> bool {
    page.origin() == setup.origin()
}

pub fn public_app_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value.trim())
        .map_err(|_| "Enter a full URL, like https://app.example.com.".to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("The app URL must start with http:// or https://.".into());
    }
    if url.host_str().is_none() {
        return Err("The app URL needs a host.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Remove the username and password from the app URL.".into());
    }
    Ok(url.to_string())
}

pub fn remembered_app_url(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(data_dir.join("app-url.txt")).ok()?;
    public_app_url(text.trim()).ok()
}

pub fn remember_app_url(data_dir: &Path, url: &str) -> Result<(), String> {
    std::fs::write(data_dir.join("app-url.txt"), url)
        .map_err(|err| format!("Could not save the app URL: {err}"))
}

pub trait ToolExecutor: Send + Sync {
    fn call(&self, name: &str, arguments: &Value) -> Result<Value, String>;

    fn page_url(&self) -> Option<String> {
        None
    }

    fn instructions(&self) -> String {
        match self.page_url() {
            Some(url) => format!("{} The open page is {url}.", base_instructions()),
            None => base_instructions(),
        }
    }
}

pub struct PageExecutor {
    app: AppHandle,
    setup_url: String,
}

impl PageExecutor {
    pub fn new(app: AppHandle, setup_url: String) -> Self {
        Self { app, setup_url }
    }
}

impl ToolExecutor for PageExecutor {
    fn call(&self, name: &str, arguments: &Value) -> Result<Value, String> {
        validate_call(name, arguments)?;
        let webview = self
            .app
            .get_webview(PAGE_WEBVIEW)
            .ok_or_else(|| "Open the app URL first.".to_string())?;
        let current = webview.url().map_err(|err| err.to_string())?;
        let setup = Url::parse(&self.setup_url).map_err(|err| err.to_string())?;
        if is_shell_page(&current, &setup) {
            return Err("Open the app URL first. These tools control that window.".into());
        }
        let id = next_id();
        webview
            .eval(invocation_script(&id, name, arguments))
            .map_err(|err| format!("Could not reach the open app: {err}"))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let raw = eval_json(&webview, &poll_script(&id))?;
            if !raw.is_empty() && raw != "null" {
                let parsed: Value = serde_json::from_str(&raw)
                    .map_err(|err| format!("The app returned an unreadable result: {err}"))?;
                return finish_call(&webview, parsed);
            }
            if Instant::now() >= deadline {
                return Err(
                    "The app did not answer. Open the app, then try the tool again.".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn page_url(&self) -> Option<String> {
        let webview = self.app.get_webview(PAGE_WEBVIEW)?;
        let current = webview.url().ok()?;
        let setup = Url::parse(&self.setup_url).ok()?;
        if is_shell_page(&current, &setup) {
            None
        } else {
            Some(current.to_string())
        }
    }
}

fn finish_call(webview: &tauri::Webview, parsed: Value) -> Result<Value, String> {
    let ok = parsed
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !ok {
        let message = parsed
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("The app tool failed.");
        return Err(message.to_string());
    }
    let mut result = parsed.get("result").cloned().unwrap_or(Value::Null);
    if let Some(target) = result
        .as_object_mut()
        .and_then(|object| object.remove("navigateTo"))
        .and_then(|value| value.as_str().map(str::to_string))
    {
        let script = format!(
            "location.assign({})",
            serde_json::to_string(&target).unwrap_or_else(|_| "\"\"".into())
        );
        let _ = webview.eval(script);
    }
    Ok(result)
}

fn eval_json(webview: &tauri::Webview, script: &str) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    webview
        .eval_with_callback(script, move |value| {
            let _ = tx.send(value);
        })
        .map_err(|err| format!("Could not reach the open app: {err}"))?;
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "The app did not answer.".to_string())
}

fn next_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("c{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn invocation_script(id: &str, name: &str, arguments: &Value) -> String {
    format!(
        "{BRIDGE}\n;(() => {{\n  const id = {id};\n  const name = {name};\n  const args = {arguments};\n  window.__appControlResults = window.__appControlResults || {{}};\n  Promise.resolve()\n    .then(() => window.__appControl.call(name, args))\n    .then((result) => {{ window.__appControlResults[id] = {{ ok: true, result }}; }})\n    .catch((error) => {{ window.__appControlResults[id] = {{ ok: false, error: String(error && error.message || error) }}; }});\n}})();\n",
        id = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into()),
        name = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into()),
        arguments = arguments,
    )
}

fn poll_script(id: &str) -> String {
    format!(
        "(function () {{ var box = window.__appControlResults; if (!box) return null; var slot = box[{id}]; if (!slot) return null; delete box[{id}]; return slot; }})()",
        id = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into()),
    )
}

pub struct ControlHandle {
    pub mcp_url: String,
    pub tools_url: String,
    pub token: String,
    server: Arc<Server>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ControlHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn serve_control(executor: Arc<dyn ToolExecutor>) -> Result<ControlHandle, String> {
    match serve_on(executor.clone(), 47_322) {
        Ok(handle) => Ok(handle),
        Err(_) => serve_on(executor, 0),
    }
}

pub fn serve_on(executor: Arc<dyn ToolExecutor>, port: u16) -> Result<ControlHandle, String> {
    let server =
        Arc::new(Server::http(format!("127.0.0.1:{port}")).map_err(|err| err.to_string())?);
    let addr = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr,
        other => return Err(format!("Unexpected listen address: {other}")),
    };
    let origin = format!("http://{addr}");
    let token = control_token();
    let session = format!("shell-{}", std::process::id());
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    let thread_server = Arc::clone(&server);
    let thread_token = token.clone();
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
            let executor = Arc::clone(&executor);
            let session = session.clone();
            let token = thread_token.clone();
            std::thread::spawn(move || {
                if let Err(err) = dispatch(request, executor.as_ref(), &session, &token) {
                    crate::journal::write("app control", &err);
                }
            });
        }
    });
    Ok(ControlHandle {
        mcp_url: format!("{origin}/mcp"),
        tools_url: format!("{origin}/tools.json"),
        token,
        server,
        shutdown,
        thread: Some(thread),
    })
}

fn dispatch(
    mut request: Request,
    executor: &dyn ToolExecutor,
    session: &str,
    token: &str,
) -> Result<(), String> {
    if request.method() == &Method::Options {
        return respond(request, 204, Vec::new(), session);
    }
    if !request_has_token(request.url(), token) {
        return respond(request, 401, Vec::new(), session);
    }
    let path = request.url().split('?').next().unwrap_or("/");
    match (request.method().clone(), path) {
        (Method::Get, "/tools.json") => {
            let body = serde_json::to_vec(&tool_catalog()).unwrap_or_else(|_| b"{}".to_vec());
            respond(request, 200, body, session)
        }
        (Method::Post, "/mcp") => {
            let mut bytes = Vec::new();
            request
                .as_reader()
                .take(1_000_000)
                .read_to_end(&mut bytes)
                .map_err(|err| err.to_string())?;
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => value,
                    Err(_) => {
                        let error = rpc_error(Value::Null, -32700, "Parse error");
                        let payload = serde_json::to_vec(&error).unwrap_or_default();
                        return respond(request, 400, payload, session);
                    }
                }
            };
            match rpc_response(&body, executor) {
                RpcOutcome::Accepted => respond(request, 202, Vec::new(), session),
                RpcOutcome::Reply(value) => {
                    let payload = serde_json::to_vec(&value).unwrap_or_default();
                    respond(request, 200, payload, session)
                }
            }
        }
        _ => respond(request, 404, Vec::new(), session),
    }
}

fn request_has_token(url: &str, token: &str) -> bool {
    let Some(query) = url.split('?').nth(1) else {
        return false;
    };
    query.split('&').any(|pair| {
        let mut parts = pair.splitn(2, '=');
        parts.next() == Some("token") && parts.next() == Some(token)
    })
}

fn control_token() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut bytes = [0u8; 32];
    let mut file = std::fs::File::open("/dev/urandom").expect("urandom");
    file.read_exact(&mut bytes).expect("urandom");
    bytes
        .into_iter()
        .map(|byte| ALPHABET[(byte as usize) % ALPHABET.len()] as char)
        .collect()
}

fn respond(request: Request, status: u16, body: Vec<u8>, session: &str) -> Result<(), String> {
    let mut response = Response::from_data(body).with_status_code(StatusCode(status));
    for (name, value) in [
        ("content-type", "application/json"),
        ("access-control-allow-origin", "*"),
        ("access-control-allow-methods", "GET, POST, OPTIONS"),
        (
            "access-control-allow-headers",
            "content-type, accept, mcp-session-id, mcp-protocol-version",
        ),
        ("access-control-expose-headers", "mcp-session-id"),
        ("mcp-session-id", session),
        ("cache-control", "no-store"),
    ] {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }
    request
        .respond(response)
        .map_err(|err| format!("Could not answer the agent: {err}"))
}

enum RpcOutcome {
    Reply(Value),
    Accepted,
}

fn rpc_response(body: &Value, executor: &dyn ToolExecutor) -> RpcOutcome {
    if let Some(items) = body.as_array() {
        let replies: Vec<Value> = items
            .iter()
            .filter_map(|item| match rpc_one(item, executor) {
                RpcOutcome::Reply(value) => Some(value),
                RpcOutcome::Accepted => None,
            })
            .collect();
        if replies.is_empty() {
            RpcOutcome::Accepted
        } else {
            RpcOutcome::Reply(Value::Array(replies))
        }
    } else {
        rpc_one(body, executor)
    }
}

fn rpc_one(body: &Value, executor: &dyn ToolExecutor) -> RpcOutcome {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body
        .get("method")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if method.starts_with("notifications/") {
        return RpcOutcome::Accepted;
    }
    let result = match method {
        "initialize" => Ok(initialize_result(body, executor)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => call_tool(body, executor),
        "" => Err(rpc_error(id.clone(), -32600, "Invalid request")),
        _ => Err(rpc_error(id.clone(), -32601, "Method not found")),
    };
    RpcOutcome::Reply(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(error) => error,
    })
}

fn call_tool(body: &Value, executor: &dyn ToolExecutor) -> Result<Value, Value> {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let params = body.get("params").cloned().unwrap_or(Value::Null);
    let name = params
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if validate_call(name, &arguments).is_err() && !known_tool(name) {
        return Err(rpc_error(id, -32602, &format!("Unknown tool {name}.")));
    }
    if let Err(message) = validate_call(name, &arguments) {
        return Ok(tool_error(&message));
    }
    match executor.call(name, &arguments) {
        Ok(value) => Ok(tool_result(value)),
        Err(message) => Ok(tool_error(&message)),
    }
}

fn tool_result(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": false
    })
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true
    })
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn initialize_result(body: &Value, executor: &dyn ToolExecutor) -> Value {
    let requested = body
        .pointer("/params/protocolVersion")
        .and_then(|value| value.as_str())
        .filter(|version| version.starts_with("2024-") || version.starts_with("2025-"))
        .unwrap_or("2024-11-05");
    json!({
        "protocolVersion": requested,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "app-shell", "version": "0.1.0" },
        "instructions": executor.instructions()
    })
}

pub fn tool_catalog() -> Value {
    json!({
        "name": "app-shell",
        "instructions": base_instructions(),
        "mcp": "/mcp",
        "tools": tool_definitions()
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut tool| {
                if let Some(schema) = tool.get("inputSchema").cloned() {
                    if let Some(object) = tool.as_object_mut() {
                        object.insert("arguments".into(), schema);
                    }
                }
                tool
            })
            .collect::<Vec<_>>()
    })
}

fn tool_definitions() -> Value {
    json!([
        tool(
            "list_pages",
            "List the pages this app can open. Use a path from this list with navigate.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })
        ),
        tool(
            "navigate",
            "Open a page in the app window. path is an app path such as /profile, or a URL on the same site.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "App path or same-site URL." }
                },
                "required": ["path"],
                "additionalProperties": false
            })
        ),
        tool(
            "describe_page",
            "Read the page that is open: its path, forms, fields, and submit buttons.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })
        ),
        tool(
            "set_fields",
            "Set values on the form that is open. fields maps a field name, id, or label to the new value. Pass form when the page has more than one form.",
            json!({
                "type": "object",
                "properties": {
                    "form": { "type": "string", "description": "Form id, name, or index. Omit when the page has one form." },
                    "fields": {
                        "type": "object",
                        "description": "Field name, id, or label to the value to set.",
                        "additionalProperties": true
                    }
                },
                "required": ["fields"],
                "additionalProperties": false
            })
        ),
        tool(
            "submit_form",
            "Do not call this. The user approves the change by clicking the confirmation button.",
            json!({
                "type": "object",
                "properties": {
                    "form": { "type": "string", "description": "Form id, name, or index. Omit when the page has one form." },
                    "button": { "type": "string", "description": "Button label. Omit to use the form's submit button." }
                },
                "additionalProperties": false
            })
        )
    ])
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema
    })
}

fn known_tool(name: &str) -> bool {
    matches!(
        name,
        "list_pages" | "navigate" | "describe_page" | "set_fields" | "submit_form"
    )
}

fn validate_call(name: &str, arguments: &Value) -> Result<(), String> {
    if !known_tool(name) {
        return Err(format!("Unknown tool {name}."));
    }
    match name {
        "navigate" => match arguments.get("path").and_then(|value| value.as_str()) {
            Some(path) if !path.trim().is_empty() => Ok(()),
            _ => Err("path is required.".into()),
        },
        "set_fields" => match arguments.get("fields") {
            Some(Value::Object(fields)) if !fields.is_empty() => Ok(()),
            _ => Err("fields must be an object of field names to values.".into()),
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{ErrorKind, Write};
    use std::net::{Shutdown, TcpStream};

    struct Stub;

    impl ToolExecutor for Stub {
        fn call(&self, name: &str, arguments: &Value) -> Result<Value, String> {
            Ok(json!({ "tool": name, "arguments": arguments }))
        }
    }

    struct OpenPage;

    impl ToolExecutor for OpenPage {
        fn call(&self, _name: &str, _arguments: &Value) -> Result<Value, String> {
            Ok(json!({}))
        }

        fn page_url(&self) -> Option<String> {
            Some("https://app.example.com/home".into())
        }
    }

    #[test]
    fn initialize_names_the_open_page() {
        let outcome = rpc_response(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2025-03-26" }
            }),
            &OpenPage,
        );
        let RpcOutcome::Reply(reply) = outcome else {
            panic!("initialize should reply");
        };
        let instructions = reply["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("list_pages"));
        assert!(instructions.contains("https://app.example.com/home"));
        assert!(instructions.contains(LEAVE_CONFIRMATION_TO_THE_USER));
        let catalog = tool_definitions();
        let submit = catalog
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "submit_form")
            .unwrap();
        assert!(
            submit["description"]
                .as_str()
                .unwrap()
                .contains("confirmation button")
        );
    }

    #[test]
    fn a_public_url_must_be_http_and_have_no_password() {
        assert!(public_app_url("https://app.example.com/home").is_ok());
        assert!(public_app_url("http://127.0.0.1:3003").is_ok());
        assert!(public_app_url("javascript:alert(1)").is_err());
        assert!(public_app_url("https://user:secret@app.example.com").is_err());
        assert!(public_app_url("not a url").is_err());
    }

    #[test]
    fn the_catalog_posts_navigation_and_form_tools() {
        let catalog = tool_definitions();
        let names: Vec<&str> = catalog
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|value| value.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                "list_pages",
                "navigate",
                "describe_page",
                "set_fields",
                "submit_form"
            ]
        );
    }

    #[test]
    fn tools_call_runs_in_the_open_app_and_rejects_a_bad_path() {
        let executor = Stub;
        let listed = rpc_response(
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list"
            }),
            &executor,
        );
        let RpcOutcome::Reply(listed) = listed else {
            panic!("tools/list should reply");
        };
        assert!(listed["result"]["tools"].as_array().unwrap().len() == 5);

        let called = rpc_response(
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "navigate", "arguments": { "path": "/profile" } }
            }),
            &executor,
        );
        let RpcOutcome::Reply(called) = called else {
            panic!("tools/call should reply");
        };
        assert_eq!(called["result"]["isError"], false);
        assert_eq!(
            called["result"]["structuredContent"]["arguments"]["path"],
            "/profile"
        );

        let missing = rpc_response(
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": { "name": "navigate", "arguments": {} }
            }),
            &executor,
        );
        let RpcOutcome::Reply(missing) = missing else {
            panic!("invalid tools/call should reply");
        };
        assert_eq!(missing["result"]["isError"], true);

        let started = rpc_response(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            &executor,
        );
        assert!(matches!(started, RpcOutcome::Accepted));
    }

    #[test]
    fn the_local_endpoint_posts_the_tool_catalog() {
        let handle = serve_on(Arc::new(Stub), 0).unwrap();
        let denied = http(&handle.tools_url, "GET", "/tools.json", None);
        assert_eq!(denied.status, 401);
        let tools = http(
            &handle.tools_url,
            "GET",
            &format!("/tools.json?token={}", handle.token),
            None,
        );
        assert_eq!(tools.status, 200);
        let catalog: Value = serde_json::from_str(&tools.body).unwrap();
        assert!(
            catalog["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "set_fields")
        );

        let body = r#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"agent","version":"0"}}}"#;
        let init = http(
            &handle.mcp_url,
            "POST",
            &format!("/mcp?token={}", handle.token),
            Some(body),
        );
        assert_eq!(init.status, 200);
        let payload: Value = serde_json::from_str(&init.body).unwrap();
        assert_eq!(payload["result"]["protocolVersion"], "2025-03-26");
        assert!(payload["result"]["capabilities"]["tools"].is_object());
        assert!(init.headers.to_ascii_lowercase().contains("mcp-session-id"));
    }

    struct Raw {
        status: u16,
        headers: String,
        body: String,
    }

    fn http(url: &str, method: &str, path: &str, body: Option<&str>) -> Raw {
        let host = url.trim_start_matches("http://").split('/').next().unwrap();
        let mut stream = TcpStream::connect(host).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let payload = body.unwrap_or("");
        write!(
            stream,
            "{method} {path} HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        )
        .unwrap();
        let _ = stream.shutdown(Shutdown::Write);
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            if message_complete(&bytes) {
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
        let (head, response_body) = text.split_once("\r\n\r\n").unwrap_or((text.as_ref(), ""));
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        Raw {
            status,
            headers: head.to_string(),
            body: response_body.to_string(),
        }
    }

    fn message_complete(bytes: &[u8]) -> bool {
        let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let head = String::from_utf8_lossy(&bytes[..split]);
        let length = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        });
        match length {
            Some(length) => bytes.len() >= split + 4 + length,
            None => false,
        }
    }
}
