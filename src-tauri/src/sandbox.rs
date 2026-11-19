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

pub fn run(code: &str, origins: &[String]) -> Result<Value, String> {
    if code.trim().is_empty() {
        return Err("script is required.".into());
    }
    if code.len() > 32 * 1024 {
        return Err("That script is too large.".into());
    }
    let hosts = net_allowlist(origins);
    if hosts.is_empty() {
        return Err("This script has no allowed network hosts.".into());
    }
    let allow = hosts.join(",");
    let child = Command::new("deno")
        .args([
            "eval",
            "--no-prompt",
            &format!("--allow-net={allow}"),
            "--ext=js",
            code,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Deno is not installed. Install Deno to run scripts.".to_string())?;
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
}
