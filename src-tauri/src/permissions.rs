use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateEvent {
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

struct PendingAsk {
    sender: std::sync::mpsc::Sender<bool>,
    name: String,
}

pub struct PermissionGate {
    pending: Mutex<HashMap<String, PendingAsk>>,
    granted: Mutex<HashSet<String>>,
    allow_all: Mutex<bool>,
    next: Mutex<u64>,
}

impl PermissionGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(HashMap::new()),
            granted: Mutex::new(HashSet::new()),
            allow_all: Mutex::new(false),
            next: Mutex::new(1),
        })
    }

    pub fn decide(name: &str, on_allowlist: bool) -> Decision {
        match name {
            "browser.snapshot" | "tabs.list" => Decision::Allow,
            "script.run" => Decision::Ask,
            "http.request" | "http.curl" => {
                if on_allowlist {
                    Decision::Allow
                } else {
                    Decision::Ask
                }
            }
            other if other.starts_with("mcp.") || other.contains('.') && !other.starts_with("browser.") && !other.starts_with("tabs.") && !other.starts_with("http.") => {
                Decision::Ask
            }
            "browser.navigate" | "browser.click" | "browser.type" | "browser.fill"
            | "tabs.open" | "tabs.focus" | "tabs.close" => Decision::Ask,
            _ => Decision::Ask,
        }
    }

    pub fn allows(&self, tool: &str) -> bool {
        if *self
            .allow_all
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            return true;
        }
        self.granted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(tool)
    }

    pub fn grant(&self, tool: &str) {
        self.granted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(tool.to_string());
    }

    pub fn grant_all(&self) {
        *self
            .allow_all
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }

    pub fn ask(
        &self,
        app: &AppHandle,
        session_id: &str,
        name: &str,
        arguments: &Value,
    ) -> bool {
        let id = {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let id = format!("gate-{}", *next);
            *next += 1;
            id
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                id.clone(),
                PendingAsk {
                    sender: tx,
                    name: name.to_string(),
                },
            );
        let _ = app.emit(
            "agent-event",
            GateEvent {
                session_id: session_id.to_string(),
                kind: "gate.ask",
                call_id: id.clone(),
                name: name.to_string(),
                arguments: arguments.clone(),
            },
        );
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(allow) => allow,
            Err(_) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id);
                false
            }
        }
    }

    pub fn reply(&self, call_id: &str, allow: bool, scope: &str) -> Result<(), String> {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(call_id);
        let Some(pending) = pending else {
            return Err("That permission prompt is no longer open.".into());
        };
        if allow && scope == "all" {
            self.grant_all();
        } else if allow && scope == "tool" {
            self.grant(&pending.name);
        }
        let _ = pending.sender.send(allow);
        Ok(())
    }
}

#[tauri::command]
pub fn gate_reply(
    gate: tauri::State<'_, Arc<PermissionGate>>,
    call_id: String,
    allow: bool,
    scope: Option<String>,
) -> Result<(), String> {
    gate.reply(&call_id, allow, scope.as_deref().unwrap_or("once"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_and_list_are_automatic() {
        assert_eq!(PermissionGate::decide("browser.snapshot", false), Decision::Allow);
        assert_eq!(PermissionGate::decide("tabs.list", false), Decision::Allow);
    }

    #[test]
    fn navigate_and_click_ask() {
        assert_eq!(PermissionGate::decide("browser.navigate", true), Decision::Ask);
        assert_eq!(PermissionGate::decide("browser.click", true), Decision::Ask);
        assert_eq!(PermissionGate::decide("script.run", true), Decision::Ask);
    }

    #[test]
    fn similar_and_all_cover_later_calls() {
        let gate = PermissionGate::new();
        assert!(!gate.allows("browser.click"));
        gate.grant("browser.click");
        assert!(gate.allows("browser.click"));
        assert!(!gate.allows("browser.type"));
        gate.grant_all();
        assert!(gate.allows("script.run"));
        assert!(gate.allows("fourss.set_fields"));
    }

    #[test]
    fn http_asks_off_the_allowlist() {
        assert_eq!(PermissionGate::decide("http.request", true), Decision::Allow);
        assert_eq!(PermissionGate::decide("http.request", false), Decision::Ask);
        assert_eq!(PermissionGate::decide("http.curl", false), Decision::Ask);
    }
}
