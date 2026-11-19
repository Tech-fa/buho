use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

use crate::http_gate;

pub fn net_allowlist(origins: &[String]) -> Vec<String> {
    let mut hosts = Vec::new();
    for origin in origins {
        if let Ok(url) = Url::parse(origin) {
            if let Some(host) = url.host_str() {
                match url.port() {
                    Some(port) => hosts.push(format!("{host}:{port}")),
                    None => hosts.push(host.to_string()),
                }
            }
        }
    }
    hosts.push("127.0.0.1".into());
    hosts.push("localhost".into());
    hosts.push("[::1]".into());
    hosts.sort();
    hosts.dedup();
    hosts
}

const PYTHON_GUARD: &str = r#"
import os, socket

def _hosts():
    raw = os.environ.get("BUHO_NET", "")
    return {item.strip().lower() for item in raw.split(",") if item.strip()}

_allowed = _hosts()

def _host_ok(host, port):
    name = str(host).lower().split("%", 1)[0].strip("[]")
    if name in {"localhost", "127.0.0.1", "::1"}:
        return True
    for item in _allowed:
        if item.startswith("[") and "]" in item:
            if name == item[1:item.index("]")]:
                return True
            continue
        if ":" in item:
            bare, _, item_port = item.rpartition(":")
            if name == bare and (port is None or str(port) == item_port):
                return True
        elif name == item:
            return True
    return False

def _check(address):
    if isinstance(address, tuple) and address:
        port = address[1] if len(address) > 1 else None
        if not _host_ok(address[0], port):
            raise PermissionError("Network to %s is not allowed." % address[0])

_socket = socket.socket
_create = socket.create_connection

class _Guard(_socket):
    def connect(self, address):
        _check(address)
        return super().connect(address)
    def connect_ex(self, address):
        _check(address)
        return super().connect_ex(address)

def _create_connection(address, *args, **kwargs):
    _check(address)
    return _create(address, *args, **kwargs)

socket.socket = _Guard
socket.create_connection = _create_connection
script = os.environ.pop("BUHO_SCRIPT", "")
exec(compile(script, "<script>", "exec"), {"__name__": "__main__"})
"#;

pub fn run(code: &str, origins: &[String]) -> Result<Value, String> {
    if code.trim().is_empty() {
        return Err("script is required.".into());
    }
    if code.len() > 32 * 1024 {
        return Err("That script is too large.".into());
    }
    if code.contains('\0') {
        return Err("That script is not valid.".into());
    }
    for word in [
        "subprocess",
        "ctypes",
        "multiprocessing",
        "os.system",
        "os.popen",
        "os.exec",
        "os.spawn",
    ] {
        if code.contains(word) {
            return Err(
                "That script cannot start another program. Use http.curl or Python's own network."
                    .into(),
            );
        }
    }
    let hosts = net_allowlist(origins);
    if hosts.is_empty() {
        return Err("This script has no allowed network hosts.".into());
    }
    let allow = hosts.join(",");
    let child = Command::new("python3")
        .args(["-I", "-c", PYTHON_GUARD])
        .env("BUHO_NET", &allow)
        .env("BUHO_SCRIPT", code)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Python is not installed. Install Python to run scripts.".to_string())?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let (stdout, truncated) = http_gate::truncate_body(&stdout);
            Ok(json!({
                "ok": output.status.success(),
                "status": output.status.code(),
                "stdout": stdout,
                "stderr": stderr,
                "truncated": truncated
            }))
        }
        Ok(Err(err)) => Err(format!("Could not run the script: {err}")),
        Err(_) => {
            kill_process(pid);
            Err("The script ran too long.".into())
        }
    }
}

fn kill_process(pid: u32) {
    #[cfg(unix)]
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status();
}

/// A script that sends a write to an API can change the open page.
pub fn script_writes(code: &str) -> bool {
    quoted_write_method(code) || call_writes(code) || urllib_posts(code)
}

fn quoted_write_method(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let quote = bytes[index];
        if quote != b'\'' && quote != b'"' {
            index += 1;
            continue;
        }
        let Some(end) = bytes[index + 1..].iter().position(|ch| *ch == quote) else {
            break;
        };
        let word = &lower[index + 1..index + 1 + end];
        if matches!(word, "post" | "put" | "patch" | "delete") {
            let before = &lower[..index];
            if introduces_write_method(before) {
                return true;
            }
        }
        index += end + 2;
    }
    false
}

fn introduces_write_method(before: &str) -> bool {
    let compact: String = before.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact.ends_with("method=") || compact.ends_with(".request(")
}

fn call_writes(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    [".post(", ".put(", ".patch(", ".delete("]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn urllib_posts(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    call_sends_body(&lower, ".request") || call_sends_body(&lower, "urlopen")
}

fn call_sends_body(code: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    let mut rest = code;
    while let Some(index) = rest.find(&needle) {
        let boundary_ok = name.starts_with('.')
            || index == 0
            || !rest.as_bytes()[index - 1].is_ascii_alphanumeric();
        let args_at = index + needle.len();
        if boundary_ok {
            if let Some(end) = close_paren(&rest[args_at..]) {
                let args = &rest[args_at..args_at + end];
                if keyword_arg(args, "data") && !quoted_read_method(args) {
                    return true;
                }
            }
        }
        rest = &rest[args_at..];
    }
    false
}

fn quoted_read_method(args: &str) -> bool {
    let bytes = args.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let quote = bytes[index];
        if quote != b'\'' && quote != b'"' {
            index += 1;
            continue;
        }
        let Some(end) = bytes[index + 1..].iter().position(|ch| *ch == quote) else {
            break;
        };
        let word = &args[index + 1..index + 1 + end];
        if matches!(word, "get" | "head" | "options") {
            return true;
        }
        index += end + 2;
    }
    false
}

fn keyword_arg(args: &str, name: &str) -> bool {
    let mut rest = args;
    while let Some(index) = rest.find(name) {
        let before_ok = index == 0 || !rest.as_bytes()[index - 1].is_ascii_alphanumeric();
        let tail = rest[index + name.len()..].trim_start();
        if before_ok && tail.starts_with('=') {
            return true;
        }
        rest = &rest[index + name.len()..];
    }
    false
}

fn close_paren(input: &str) -> Option<usize> {
    let mut depth = 1;
    let mut chars = input.char_indices();
    let mut quote: Option<char> = None;
    while let Some((index, ch)) = chars.next() {
        if let Some(active) = quote {
            if ch == '\\' {
                chars.next();
                continue;
            }
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn run_json(arguments: &Value, origins: &[String]) -> Result<Value, String> {
    let code = arguments
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| arguments.get("script").and_then(Value::as_str))
        .ok_or_else(|| "code is required.".to_string())?;
    run(code, origins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_uses_the_same_hosts_as_http() {
        let origins = http_gate::origins_from_instance(
            Some("https://app.example.com"),
            Some("http://203.0.113.10:3012"),
        );
        let hosts = net_allowlist(&origins);
        assert!(hosts.iter().any(|host| host == "app.example.com"));
        assert!(hosts.iter().any(|host| host == "203.0.113.10:3012"));
        assert!(hosts.iter().any(|host| host == "localhost"));
        assert!(!hosts.iter().any(|host| host.contains("evil")));
    }

    #[test]
    fn python_prints_and_cannot_leave_the_allowlist() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let printed = run("print('hello')", &[]).unwrap();
        assert!(printed["stdout"].as_str().unwrap_or("").contains("hello"));
        let blocked = run(
            "import urllib.request\nurllib.request.urlopen('http://203.0.113.10/secret')\n",
            &[],
        )
        .unwrap();
        assert_eq!(blocked["ok"], json!(false));
        let detail = format!(
            "{}{}",
            blocked["stdout"].as_str().unwrap_or(""),
            blocked["stderr"].as_str().unwrap_or("")
        );
        assert!(detail.contains("not allowed") || detail.contains("PermissionError"));
    }

    #[test]
    fn a_script_cannot_start_another_program() {
        assert!(run("import subprocess", &[]).is_err());
    }

    #[test]
    fn a_write_reloads_and_a_read_does_not() {
        assert!(script_writes("requests.post(url, json=body)"));
        assert!(script_writes("requests.put(url, json=body)"));
        assert!(script_writes("requests.patch(url, json=body)"));
        assert!(script_writes("requests.delete(url)"));
        assert!(script_writes(
            "urllib.request.Request(url, data=body, method='POST')"
        ));
        assert!(script_writes("conn.request(\"DELETE\", path)"));
        assert!(script_writes("urllib.request.Request(url, data=body)"));
        assert!(script_writes("urllib.request.urlopen(url, data=body)"));
        assert!(!script_writes("requests.get(url)"));
        assert!(!script_writes("print('hello')"));
        assert!(!script_writes("print('post')"));
        assert!(!script_writes(
            "urllib.request.urlopen('http://127.0.0.1:3002/health')"
        ));
        assert!(!script_writes(
            "req = urllib.request.Request(url, method='GET')\nwith urllib.request.urlopen(req) as response:\n    data = response.read()\n"
        ));
        assert!(!script_writes(
            "urllib.request.Request(url, data=body, method=\"GET\")"
        ));
    }
}
