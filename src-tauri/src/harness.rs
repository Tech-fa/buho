use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::host;
use crate::tabs::Workspace;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

#[derive(Deserialize)]
struct SidecarLine {
    op: String,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    event: Option<Value>,
    id: Option<String>,
    name: Option<String>,
    arguments: Option<Value>,
}

pub struct Harness {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    start_error: Mutex<Option<String>>,
}

impl Harness {
    pub fn spawn(app: AppHandle) -> Arc<Self> {
        let harness = Arc::new(Self {
            stdin: Mutex::new(None),
            child: Mutex::new(None),
            reader: Mutex::new(None),
            start_error: Mutex::new(None),
        });
        let Some(script) = script_path(&app) else {
            harness.note_failure("The agent script is missing from the app.");
            return harness;
        };
        let Some(node) = bundled_node() else {
            harness.note_failure("The app is missing its Node runtime.");
            return harness;
        };
        match Command::new(&node)
            .arg(&script)
            .current_dir(script.parent().unwrap_or(std::path::Path::new(".")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env("BUHO_LLM_KEY", crate::llm_key::key())
            .spawn()
        {
            Ok(mut child) => {
                let stdout = child.stdout.take();
                let stdin = child.stdin.take();
                *harness
                    .child
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(child);
                *harness
                    .stdin
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = stdin;
                if let Some(stdout) = stdout {
                    let handle = app.clone();
                    let thread = std::thread::spawn(move || {
                        let reader = BufReader::new(stdout);
                        for line in reader.lines() {
                            let Ok(line) = line else { break };
                            if line.trim().is_empty() {
                                continue;
                            }
                            if line.len() > 1_000_000 {
                                crate::journal::write("harness", "sidecar line was too large");
                                continue;
                            }
                            handle_line(&handle, &line);
                        }
                    });
                    *harness
                        .reader
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(thread);
                }
            }
            Err(err) => {
                harness.note_failure(format!("Could not start the agent: {err}"));
            }
        }
        harness
    }

    fn note_failure(&self, message: impl Into<String>) {
        let message = message.into();
        crate::journal::write("harness", &message);
        *self
            .start_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(message);
    }

    fn unavailable(&self) -> String {
        self.start_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_else(|| "The agent is not running.".into())
    }

    fn send(&self, value: &Value) -> bool {
        let mut slot = self
            .stdin
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(stdin) = slot.as_mut() else {
            return false;
        };
        let mut line = value.to_string();
        line.push('\n');
        if stdin.write_all(line.as_bytes()).is_err() || stdin.flush().is_err() {
            *slot = None;
            return false;
        }
        true
    }

    pub fn connect_mcp(&self, tab_id: &str, name: &str, url: &str) {
        let _ = self.send(&json!({
            "op": "mcp.connect",
            "tabId": tab_id,
            "name": name,
            "url": url
        }));
    }

    pub fn disconnect_mcp(&self, tab_id: &str, name: &str) {
        let _ = self.send(&json!({
            "op": "mcp.disconnect",
            "tabId": tab_id,
            "name": name
        }));
    }

    pub fn stop(&self, session_id: &str) -> bool {
        self.send(&json!({ "op": "stop", "sessionId": session_id }))
    }

    pub fn set_key(&self, key: &str) -> bool {
        self.send(&json!({ "op": "llm.key", "key": key }))
    }

    pub fn turn(
        &self,
        session_id: &str,
        text: &str,
        tab_id: Option<&str>,
        mcp: Value,
        provider: &str,
        history: &Value,
    ) -> bool {
        self.send(&json!({
            "op": "turn",
            "sessionId": session_id,
            "text": text,
            "tabId": tab_id,
            "provider": provider,
            "history": history,
            "preamble": host::HOST_INSTRUCTIONS,
            "hostTools": crate::control::tool_definitions(),
            "mcp": mcp,
            "maxTurns": 200
        }))
    }

    pub fn alive(&self) -> bool {
        self.stdin
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

fn node_file_name() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

fn node_beside(executable: &Path) -> Option<PathBuf> {
    let path = executable.parent()?.join(node_file_name());
    path.is_file().then_some(path)
}

fn bundled_node() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(node_beside)
}

fn script_path(app: &AppHandle) -> Option<PathBuf> {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../harness/server.mjs");
    if dev.exists() {
        return Some(dev);
    }
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("harness/server.mjs"))
        .filter(|path| path.exists())
}

fn handle_line(app: &AppHandle, line: &str) {
    let parsed: SidecarLine = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(err) => {
            crate::journal::write("harness", &format!("bad sidecar line: {err}"));
            return;
        }
    };
    match parsed.op.as_str() {
        "event" => {
            if let Some(event) = parsed.event {
                let kind = event
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let session = event
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if kind == "context" {
                    let messages = event
                        .get("messages")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let model = event.get("model").and_then(Value::as_str).unwrap_or("");
                    if !session.is_empty() {
                        if let Some(shell) = app.try_state::<crate::Shell>() {
                            let saved = crate::chats::save_context(
                                &crate::chats::dir(shell.data_dir()),
                                &session,
                                &messages,
                                "deepseek",
                                model,
                            );
                            drop(shell);
                            if let Ok(saved) = saved {
                                crate::tabs::note_chat_title(app, &session, &saved.title);
                            }
                        }
                    }
                    return;
                }
                if matches!(kind.as_str(), "session.result" | "session.error")
                    && !session.is_empty()
                {
                    if let Some(workspace) = app.try_state::<Arc<Workspace>>() {
                        workspace.with(|manager| manager.unlock_session(&session));
                    }
                }
                let _ = app.emit("agent-event", event);
            }
        }
        "host.call" => {
            let id = parsed.id.unwrap_or_default();
            let name = parsed.name.unwrap_or_default();
            let arguments = parsed.arguments.unwrap_or_else(|| json!({}));
            let session = parsed.session_id.clone().unwrap_or_default();
            let handle = app.clone();
            std::thread::spawn(move || {
                let result = host::dispatch(&handle, Some(&session), &name, &arguments);
                let reply = match result {
                    Ok(value) if value.get("denied").and_then(Value::as_bool) == Some(true) => {
                        json!({
                            "op": "host.result",
                            "id": id,
                            "ok": false,
                            "denied": true,
                            "error": value.get("message").and_then(Value::as_str).unwrap_or("Denied.")
                        })
                    }
                    Ok(value) => json!({
                        "op": "host.result",
                        "id": id,
                        "ok": true,
                        "result": value
                    }),
                    Err(message) => json!({
                        "op": "host.result",
                        "id": id,
                        "ok": false,
                        "error": message
                    }),
                };
                if let Some(harness) = handle.try_state::<Arc<Harness>>() {
                    harness.send(&reply);
                }
            });
        }
        _ => {}
    }
}

fn emit(app: &AppHandle, event: AgentEvent) {
    let _ = app.emit("agent-event", event);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSendOutcome {
    pub ok: bool,
    pub message: Option<String>,
    pub needs_key: bool,
}

impl AgentSendOutcome {
    fn fail(message: &str, needs_key: bool) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
            needs_key,
        }
    }

    fn sent() -> Self {
        Self {
            ok: true,
            message: None,
            needs_key: false,
        }
    }
}

#[tauri::command]
pub fn agent_send(
    app: AppHandle,
    harness: State<'_, Arc<Harness>>,
    shell: State<'_, crate::Shell>,
    session_id: String,
    text: String,
) -> Result<AgentSendOutcome, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(AgentSendOutcome::fail("Type a message first.", false));
    }
    if !crate::tabs::llm_key_set() {
        return Ok(AgentSendOutcome::fail(
            "Paste a buho_llm token to talk to the model.",
            true,
        ));
    }
    if !harness.alive() {
        return Ok(AgentSendOutcome::fail(&harness.unavailable(), false));
    }
    let folder = crate::chats::dir(shell.data_dir());
    let (agent_id, tab_id, mcp) = app.state::<Arc<Workspace>>().with(|manager| {
        let agent_id = manager
            .agent_by_session(&session_id)
            .map(|tab| tab.id.clone())
            .unwrap_or_else(|| session_id.clone());
        let tab_id = manager.default_tab_id(&session_id);
        let mcp = tab_id
            .as_ref()
            .and_then(|id| manager.offer(id))
            .map(|offer| json!([{ "name": offer.name, "url": offer.url, "tabId": tab_id }]))
            .unwrap_or_else(|| json!([]));
        (agent_id, tab_id, mcp)
    });
    let saved = crate::chats::append_user(&folder, &agent_id, &session_id, text, "deepseek")?;
    crate::tabs::note_chat_title(&app, &session_id, &saved.title);
    if !harness.turn(
        &session_id,
        text,
        tab_id.as_deref(),
        mcp,
        "deepseek",
        &Value::Array(saved.messages),
    ) {
        return Ok(AgentSendOutcome::fail("The agent is not running.", false));
    }
    Ok(AgentSendOutcome::sent())
}

#[tauri::command]
pub fn agent_stop(
    app: AppHandle,
    harness: State<'_, Arc<Harness>>,
    session_id: String,
) -> Result<(), String> {
    let sent = harness.stop(&session_id);
    app.state::<Arc<Workspace>>()
        .with(|manager| manager.unlock_session(&session_id));
    if !sent {
        emit(
            &app,
            AgentEvent {
                session_id,
                kind: "session.result".into(),
                text: Some("Stopped.".into()),
                name: None,
                arguments: None,
                call_id: None,
                ok: Some(true),
                truncated: None,
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "buho-node-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn the_runtime_next_to_the_app_is_the_one_that_starts() {
        let dir = scratch();
        let app = dir.join("desktop-shell");
        fs::write(&app, "").unwrap();
        let runtime = dir.join(node_file_name());
        fs::write(&runtime, "").unwrap();
        assert_eq!(node_beside(&app), Some(runtime));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_machine_with_node_elsewhere_does_not_supply_the_runtime() {
        let dir = scratch();
        let app = dir.join("desktop-shell");
        fs::write(&app, "").unwrap();
        assert_eq!(node_beside(&app), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
