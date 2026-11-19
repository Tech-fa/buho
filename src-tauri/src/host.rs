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
            let url = crate::engine::typed_address(url)?;
            refuse_api_page(app, &url, None)?;
            tabs::open_tab(app, &url)
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
            let page = page_url_for(app, &id);
            refuse_api_page(app, &url, page.as_deref())?;
            tabs::navigate_tab(app, &id, &url)
        }
        "browser.snapshot" | "browser.click" | "browser.type" | "browser.fill" => {
            let id = tab_id.ok_or_else(|| "Open a page first.".to_string())?;
            invoke_bridge(app, &id, name, arguments)
        }
        "http.request" | "http.curl" => {
            let writes = call_writes(name, arguments);
            let result = if name == "http.request" {
                http_gate::request_json(arguments, &allowlist(app))
            } else {
                http_gate::curl_json(arguments, &allowlist(app))
            };
            note_page_refresh(app, writes, result)
        }
        "script.run" => {
            let writes = sandbox::script_writes(script_code(arguments));
            let result = sandbox::run_json(arguments, &allowlist(app));
            note_page_refresh(app, writes, result)
        }
        _ => Err(format!("Unknown tool {name}.")),
    }
}

fn refuse_api_page(app: &AppHandle, url: &str, page: Option<&str>) -> Result<(), String> {
    let Ok(parsed) = url::Url::parse(url) else {
        return Ok(());
    };
    let api = app
        .try_state::<Arc<Workspace>>()
        .and_then(|workspace| workspace.api_origin());
    if http_gate::browser_calls_api(&parsed, api.as_deref(), page) {
        return Err(
            "Do not call an API from the browser. Use http.curl or script.run with Python.".into(),
        );
    }
    Ok(())
}

fn page_url_for(app: &AppHandle, tab_id: &str) -> Option<String> {
    app.try_state::<Arc<Workspace>>()?
        .with(|manager| {
            manager
                .list()
                .iter()
                .find(|tab| tab.id == tab_id)
                .map(|tab| tab.url.clone())
        })
        .filter(|url| !url.is_empty())
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

fn call_method(name: &str, arguments: &Value) -> Option<String> {
    match name {
        "http.request" => Some(
            arguments
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .trim()
                .to_ascii_uppercase(),
        ),
        "http.curl" => {
            let command = arguments
                .get("command")
                .and_then(Value::as_str)
                .or_else(|| arguments.get("curl").and_then(Value::as_str))?;
            http_gate::parse_curl(command).ok().map(|req| req.method)
        }
        _ => None,
    }
}

fn call_writes(name: &str, arguments: &Value) -> bool {
    call_method(name, arguments).is_some_and(|method| http_gate::writes_data(&method))
}

fn script_code(arguments: &Value) -> &str {
    arguments
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| arguments.get("script").and_then(Value::as_str))
        .unwrap_or("")
}

fn should_reload(writes: bool, value: &Value) -> bool {
    writes && value.get("ok").and_then(Value::as_bool) == Some(true)
}

fn note_page_refresh(
    app: &AppHandle,
    writes: bool,
    result: Result<Value, String>,
) -> Result<Value, String> {
    let mut value = result?;
    if should_reload(writes, &value) && reload_focused_page(app) {
        if let Some(object) = value.as_object_mut() {
            object.insert("pageRefreshed".to_string(), json!(true));
        }
    }
    Ok(value)
}

fn reload_focused_page(app: &AppHandle) -> bool {
    let Some(id) = app.try_state::<Arc<Workspace>>().and_then(|workspace| {
        workspace.with(|manager| {
            manager
                .focused()
                .filter(|tab| !tab.url.trim().is_empty())
                .map(|tab| tab.id.clone())
        })
    }) else {
        return false;
    };
    match tabs::reload_tab(app, &id) {
        Ok(reloaded) => reloaded,
        Err(err) => {
            crate::journal::write("page refresh", &err);
            false
        }
    }
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
    if name.contains('.')
        && !name.starts_with("browser.")
        && !name.starts_with("tabs.")
        && !name.starts_with("http.")
        && name != "script.run"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_write_reloads_the_open_page() {
        let created = json!({ "ok": true, "status": 201 });
        let rejected = json!({ "ok": false, "status": 422 });
        let read = json!({
            "method": "GET",
            "url": "https://api.example.com/items"
        });
        let write = json!({
            "method": "post",
            "url": "https://api.example.com/items"
        });
        let curl = json!({
            "command": "curl https://api.example.com/items --data '{\"name\":\"a\"}'"
        });
        assert!(should_reload(call_writes("http.request", &write), &created));
        assert!(!should_reload(
            call_writes("http.request", &write),
            &rejected
        ));
        assert!(!should_reload(call_writes("http.request", &read), &created));
        assert!(should_reload(call_writes("http.curl", &curl), &created));
        assert!(should_reload(
            sandbox::script_writes("requests.post(url, json={'name': 'a'})"),
            &json!({ "ok": true, "status": 0 })
        ));
        assert!(!should_reload(
            sandbox::script_writes("requests.get(url)"),
            &json!({ "ok": true, "status": 0 })
        ));
    }
}
