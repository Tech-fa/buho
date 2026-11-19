use std::sync::Arc;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::control::{ToolExecutor, invoke_bridge};
use crate::http_gate;
use crate::permissions::{Decision, PermissionGate};
use crate::sandbox;
use crate::tabs::{self, Workspace};

pub use crate::control::HOST_INSTRUCTIONS;

pub struct HostExecutor {
    app: AppHandle,
}

impl HostExecutor {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ToolExecutor for HostExecutor {
    fn call(&self, name: &str, arguments: &Value) -> Result<Value, String> {
        dispatch(&self.app, None, name, arguments)
    }

    fn instructions(&self) -> String {
        match page_url(&self.app) {
            Some(url) => format!("{} The open page is {url}.", HOST_INSTRUCTIONS),
            None => HOST_INSTRUCTIONS.to_string(),
        }
    }

    fn page_url(&self) -> Option<String> {
        page_url(&self.app)
    }
}

fn page_url(app: &AppHandle) -> Option<String> {
    app.try_state::<Arc<Workspace>>()?
        .with(|manager| manager.focused().map(|tab| tab.url.clone()))
}

pub fn dispatch(
    app: &AppHandle,
    session_id: Option<&str>,
    name: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let tab_id = tab_id_for(app, session_id, arguments);
    if let Some(session) = session_id {
        if let Some(tab) = tab_id.as_deref() {
            if matches!(
                name,
                "browser.navigate"
                    | "browser.click"
                    | "browser.type"
                    | "browser.fill"
                    | "tabs.open"
            ) {
                app.state::<Arc<Workspace>>()
                    .with(|manager| manager.lock_tab(tab, session))?;
            }
        }
        if !permit(app, session, name, arguments, tab_id.as_deref())? {
            return Ok(json!({ "ok": false, "denied": true, "message": "Denied." }));
        }
        if !is_host_tool(name) {
            return Ok(json!({ "ok": true, "permitted": true }));
        }
    }
    match name {
        "tabs.list" => Ok(tabs::list_tabs(app)),
        "tabs.open" => {
            let url = arguments
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| "url is required.".to_string())?;
            tabs::open_tab(app, url)
        }
        "tabs.focus" => {
            let id = arguments
                .get("id")
                .and_then(Value::as_str)
                .or(tab_id.as_deref())
                .ok_or_else(|| "id is required.".to_string())?;
            tabs::focus_tab(app, id)
        }
        "tabs.close" => {
            let id = arguments
                .get("id")
                .and_then(Value::as_str)
                .or(tab_id.as_deref())
                .ok_or_else(|| "id is required.".to_string())?;
            tabs::close_tab(app, id)
        }
        "browser.navigate" => {
            let raw = arguments
                .get("url")
                .and_then(Value::as_str)
                .or_else(|| arguments.get("path").and_then(Value::as_str))
                .ok_or_else(|| "url is required.".to_string())?;
            let id = tab_id.ok_or_else(|| "Open a page first.".to_string())?;
            let url = resolve_page_url(app, &id, raw)?;
            tabs::navigate_tab(app, &id, &url)
        }
        "browser.snapshot" | "browser.click" | "browser.type" | "browser.fill" => {
            let id = tab_id.ok_or_else(|| "Open a page first.".to_string())?;
            invoke_bridge(app, &id, name, arguments)
        }
        "http.request" => http_gate::request_json(arguments, &allowlist(app)),
        "http.curl" => http_gate::curl_json(arguments, &allowlist(app)),
        "script.run" => sandbox::run_json(arguments, &allowlist(app)),
        _ => Err(format!("Unknown tool {name}.")),
    }
}

fn resolve_page_url(app: &AppHandle, tab_id: &str, raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("url is required.".into());
    }
    if let Ok(url) = tabs::public_http_url(trimmed) {
        return Ok(url);
    }
    let base = app
        .try_state::<Arc<Workspace>>()
        .and_then(|workspace| {
            workspace.with(|manager| {
                manager
                    .list()
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .map(|tab| tab.url.clone())
            })
        })
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "Open a page first.".to_string())?;
    let joined = url::Url::parse(&base)
        .map_err(|_| "The open page address is not valid.".to_string())?
        .join(trimmed)
        .map_err(|_| "url is required.".to_string())?;
    tabs::public_http_url(joined.as_str())
}

fn is_host_tool(name: &str) -> bool {
    matches!(
        name,
        "tabs.list"
            | "tabs.open"
            | "tabs.focus"
            | "tabs.close"
            | "browser.navigate"
            | "browser.snapshot"
            | "browser.click"
            | "browser.type"
            | "browser.fill"
            | "http.request"
            | "http.curl"
            | "script.run"
    )
}

fn tab_id_for(app: &AppHandle, session_id: Option<&str>, arguments: &Value) -> Option<String> {
    if let Some(id) = arguments.get("tabId").and_then(Value::as_str) {
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    let workspace = app.try_state::<Arc<Workspace>>()?;
    workspace.with(|manager| {
        if let Some(session) = session_id {
            manager.default_tab_id(session)
        } else {
            manager.focused().map(|tab| tab.id.clone())
        }
    })
}

fn allowlist(app: &AppHandle) -> Vec<String> {
    app.try_state::<Arc<Workspace>>()
        .map(|workspace| workspace.origins())
        .unwrap_or_else(|| http_gate::origins_from_instance(None, None))
}

fn permit(
    app: &AppHandle,
    session_id: &str,
    name: &str,
    arguments: &Value,
    tab_id: Option<&str>,
) -> Result<bool, String> {
    let on_allowlist = match name {
        "http.request" => url_on_allowlist(app, arguments.get("url").and_then(Value::as_str)),
        "http.curl" => curl_on_allowlist(app, arguments),
        _ => true,
    };
    let Some(gate) = app.try_state::<Arc<PermissionGate>>() else {
        return Ok(true);
    };
    if gate.allows(name) {
        return Ok(true);
    }
    if name.contains('.') && !name.starts_with("browser.") && !name.starts_with("tabs.") && !name.starts_with("http.") && name != "script.run"
    {
        return Ok(gate.ask(app, session_id, name, arguments));
    }
    match PermissionGate::decide(name, on_allowlist) {
        Decision::Allow => Ok(true),
        Decision::Deny => Ok(false),
        Decision::Ask => {
            let mut shown = arguments.clone();
            if let Some(tab) = tab_id {
                if let Some(object) = shown.as_object_mut() {
                    object.entry("tabId").or_insert_with(|| json!(tab));
                }
            }
            Ok(gate.ask(app, session_id, name, &shown))
        }
    }
}

fn url_on_allowlist(app: &AppHandle, url: Option<&str>) -> bool {
    let Some(url) = url.and_then(|value| url::Url::parse(value).ok()) else {
        return false;
    };
    http_gate::allowed(&url, &allowlist(app))
}

fn curl_on_allowlist(app: &AppHandle, arguments: &Value) -> bool {
    let command = arguments
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| arguments.get("curl").and_then(Value::as_str));
    let Some(command) = command else {
        return false;
    };
    match http_gate::parse_curl(command) {
        Ok(req) => url_on_allowlist(app, Some(&req.url)),
        Err(_) => false,
    }
}
