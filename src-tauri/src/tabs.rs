use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Webview, WebviewUrl,
};
#[cfg(not(target_os = "macos"))]
use tauri::{Position, Rect, Size};
use url::Url;

use crate::control::{bridge_script, is_page_webview};
use crate::engine::{
    HistoryAction, align_page, enable_gestures, navigation_allowed, page_url_from_engine,
    parse_history_action, read_history_buttons, run_history, typed_address,
};

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub kind: &'static str,
    pub id: String,
    pub instance_id: Option<String>,
    pub url: String,
    pub title: String,
    pub webview_label: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTab {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub session_id: String,
    pub bound_tab_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub instance_id: String,
    pub browser_tabs: Vec<BrowserTab>,
    pub agent_tabs: Vec<AgentTab>,
    pub active_browser_id: Option<String>,
    pub active_agent_id: Option<String>,
    pub llm_key_set: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpOffer {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Default)]
pub struct TabManager {
    browser: Vec<BrowserTab>,
    agents: Vec<AgentTab>,
    focused: Option<String>,
    focused_agent: Option<String>,
    next_page: u32,
    next_agent: u32,
    instance_id: String,
    bounds: Bounds,
    origins: Vec<String>,
    api_origin: Option<String>,
    offers: BTreeMap<String, McpOffer>,
    busy: BTreeMap<String, String>,
    pending_mcp: BTreeMap<String, String>,
    visible: bool,
    page_covered: bool,
}

impl TabManager {
    pub fn open(&mut self, url: &str, instance_id: Option<String>) -> Result<BrowserTab, String> {
        let url = typed_address(url)?;
        self.next_page += 1;
        let id = format!("page-{}", self.next_page);
        let host = Url::parse(&url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_else(|| url.clone());
        let tab = BrowserTab {
            kind: "browser",
            id: id.clone(),
            instance_id,
            url: url.clone(),
            title: host,
            webview_label: id.clone(),
            loading: true,
            can_go_back: false,
            can_go_forward: false,
        };
        self.browser.push(tab.clone());
        self.focused = Some(id);
        Ok(tab)
    }

    pub fn open_blank(&mut self) -> BrowserTab {
        self.next_page += 1;
        let id = format!("page-{}", self.next_page);
        let tab = BrowserTab {
            kind: "browser",
            id: id.clone(),
            instance_id: None,
            url: String::new(),
            title: "New tab".into(),
            webview_label: id.clone(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        };
        self.browser.push(tab.clone());
        self.focused = Some(id);
        tab
    }

    pub fn focus(&mut self, id: &str) -> Result<BrowserTab, String> {
        let tab = self
            .browser
            .iter()
            .find(|tab| tab.id == id)
            .cloned()
            .ok_or_else(|| "That tab is not open.".to_string())?;
        self.focused = Some(id.to_string());
        Ok(tab)
    }

    pub fn close(&mut self, id: &str) -> Result<BrowserTab, String> {
        if self.browser.len() <= 1 {
            return Err("Keep at least one tab.".into());
        }
        self.discard(id)
    }

    fn discard(&mut self, id: &str) -> Result<BrowserTab, String> {
        let index = self
            .browser
            .iter()
            .position(|tab| tab.id == id)
            .ok_or_else(|| "That tab is not open.".to_string())?;
        let tab = self.browser.remove(index);
        self.offers.remove(id);
        self.busy.remove(id);
        self.pending_mcp.remove(id);
        if self.focused.as_deref() == Some(id) {
            self.focused = self
                .browser
                .get(index)
                .or_else(|| self.browser.last())
                .map(|tab| tab.id.clone());
        }
        for agent in &mut self.agents {
            agent.bound_tab_ids.retain(|bound| bound != id);
        }
        Ok(tab)
    }

    pub fn list(&self) -> &[BrowserTab] {
        &self.browser
    }

    pub fn focused(&self) -> Option<&BrowserTab> {
        let id = self.focused.as_deref()?;
        self.browser.iter().find(|tab| tab.id == id)
    }

    pub fn set_url(&mut self, id: &str, url: &str, title: Option<&str>) {
        self.apply_engine(
            id,
            page_url_from_engine(url).as_deref().or(Some(url)),
            title,
            None,
            None,
            None,
        );
    }

    pub fn apply_engine(
        &mut self,
        id: &str,
        url: Option<&str>,
        title: Option<&str>,
        loading: Option<bool>,
        can_go_back: Option<bool>,
        can_go_forward: Option<bool>,
    ) -> bool {
        let Some(tab) = self.browser.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        let mut changed = false;
        if let Some(url) = url.and_then(page_url_from_engine) {
            if tab.url != url {
                tab.url = url;
                changed = true;
            }
        }
        if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
            if tab.title != title {
                tab.title = title.to_string();
                changed = true;
            }
        }
        if let Some(loading) = loading {
            if tab.loading != loading {
                tab.loading = loading;
                changed = true;
            }
        }
        if let Some(can_go_back) = can_go_back {
            if tab.can_go_back != can_go_back {
                tab.can_go_back = can_go_back;
                changed = true;
            }
        }
        if let Some(can_go_forward) = can_go_forward {
            if tab.can_go_forward != can_go_forward {
                tab.can_go_forward = can_go_forward;
                changed = true;
            }
        }
        changed
    }

    pub fn new_agent(&mut self) -> AgentTab {
        self.next_agent += 1;
        let n = self.next_agent;
        let bound = self.focused.clone().into_iter().collect();
        let tab = AgentTab {
            kind: "agent",
            id: format!("agent-{n}"),
            title: "New chat".into(),
            session_id: format!("session-{n}"),
            bound_tab_ids: bound,
        };
        self.agents.push(tab.clone());
        self.focused_agent = Some(tab.id.clone());
        tab
    }

    pub fn close_agent(&mut self, id: &str) -> Result<AgentTab, String> {
        let index = self
            .agents
            .iter()
            .position(|tab| tab.id == id)
            .ok_or_else(|| "That chat is not open.".to_string())?;
        let tab = self.agents.remove(index);
        if self.focused_agent.as_deref() == Some(id) {
            self.focused_agent = self
                .agents
                .get(index)
                .or_else(|| self.agents.last())
                .map(|tab| tab.id.clone());
        }
        Ok(tab)
    }

    pub fn restore_open(&mut self, records: &[crate::chats::ChatRecord]) {
        for record in records.iter().filter(|record| record.open) {
            if let Some(number) = record
                .id
                .strip_prefix("agent-")
                .and_then(|value| value.parse::<u32>().ok())
            {
                self.next_agent = self.next_agent.max(number);
            }
            if self
                .agents
                .iter()
                .any(|tab| tab.session_id == record.session_id)
            {
                continue;
            }
            self.agents.push(AgentTab {
                kind: "agent",
                id: record.id.clone(),
                title: record.title.clone(),
                session_id: record.session_id.clone(),
                bound_tab_ids: Vec::new(),
            });
            self.focused_agent = Some(record.id.clone());
        }
    }

    pub fn open_saved(&mut self, record: &crate::chats::ChatRecord) -> AgentTab {
        if let Some(existing) = self
            .agents
            .iter()
            .find(|tab| tab.session_id == record.session_id)
            .cloned()
        {
            self.focused_agent = Some(existing.id.clone());
            return existing;
        }
        if let Some(number) = record
            .id
            .strip_prefix("agent-")
            .and_then(|value| value.parse::<u32>().ok())
        {
            self.next_agent = self.next_agent.max(number);
        }
        let tab = AgentTab {
            kind: "agent",
            id: record.id.clone(),
            title: record.title.clone(),
            session_id: record.session_id.clone(),
            bound_tab_ids: self.focused.clone().into_iter().collect(),
        };
        self.agents.push(tab.clone());
        self.focused_agent = Some(tab.id.clone());
        tab
    }

    pub fn rename_session(&mut self, session_id: &str, title: &str) -> bool {
        let Some(agent) = self
            .agents
            .iter_mut()
            .find(|tab| tab.session_id == session_id)
        else {
            return false;
        };
        if title.trim().is_empty() || agent.title == title {
            return false;
        }
        agent.title = title.to_string();
        true
    }

    pub fn focus_agent(&mut self, id: &str) -> Result<AgentTab, String> {
        let tab = self
            .agents
            .iter()
            .find(|tab| tab.id == id)
            .cloned()
            .ok_or_else(|| "That agent is not open.".to_string())?;
        self.focused_agent = Some(id.to_string());
        Ok(tab)
    }

    pub fn bind_focused_agent(&mut self) {
        let focused = self.focused.clone();
        let Some(agent_id) = self.focused_agent.clone() else {
            return;
        };
        if let Some(agent) = self.agents.iter_mut().find(|tab| tab.id == agent_id) {
            agent.bound_tab_ids = focused.into_iter().collect();
        }
    }

    pub fn agent_by_session(&self, session_id: &str) -> Option<&AgentTab> {
        self.agents.iter().find(|tab| tab.session_id == session_id)
    }

    pub fn focused_agent_session(&self) -> Option<String> {
        let id = self.focused_agent.as_deref()?;
        self.agents
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.session_id.clone())
            .or_else(|| self.agents.first().map(|tab| tab.session_id.clone()))
    }

    pub fn default_tab_id(&self, session_id: &str) -> Option<String> {
        if let Some(agent) = self.agent_by_session(session_id) {
            if let Some(id) = agent.bound_tab_ids.first() {
                return Some(id.clone());
            }
        }
        self.focused.clone()
    }

    pub fn lock_tab(&mut self, tab_id: &str, session_id: &str) -> Result<(), String> {
        if let Some(owner) = self.busy.get(tab_id) {
            if owner != session_id {
                return Err("That page is already in use by another agent.".into());
            }
        }
        self.busy.insert(tab_id.to_string(), session_id.to_string());
        Ok(())
    }

    pub fn unlock_session(&mut self, session_id: &str) {
        self.busy.retain(|_, owner| owner != session_id);
    }

    pub fn set_offer(&mut self, tab_id: &str, offer: Option<McpOffer>) -> bool {
        match offer {
            Some(offer) => self.offers.insert(tab_id.to_string(), offer).is_none(),
            None => self.offers.remove(tab_id).is_some(),
        }
    }

    pub fn offer(&self, tab_id: &str) -> Option<&McpOffer> {
        self.offers.get(tab_id)
    }

    pub fn state(&self) -> WorkspaceState {
        WorkspaceState {
            instance_id: self.instance_id.clone(),
            browser_tabs: self.browser.clone(),
            agent_tabs: self.agents.clone(),
            active_browser_id: self.focused.clone(),
            active_agent_id: self.focused_agent.clone(),
            llm_key_set: llm_key_set(),
        }
    }
}

pub fn bounds_ready(bounds: Bounds) -> bool {
    bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width >= 48.0
        && bounds.height >= 48.0
}

pub fn public_http_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value.trim())
        .map_err(|_| "Enter a full URL, like https://example.com.".to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("The page URL must start with http:// or https://.".into());
    }
    if url.host_str().is_none() {
        return Err("The page URL needs a host.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Remove the username and password from the URL.".into());
    }
    Ok(url.to_string())
}

pub fn llm_key_set() -> bool {
    crate::llm_key::is_set()
}

pub(crate) fn publish_state(app: &AppHandle) {
    emit_state(app);
}

pub struct Workspace {
    inner: Mutex<TabManager>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(TabManager::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TabManager> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn snapshot(&self) -> WorkspaceState {
        self.lock().state()
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut TabManager) -> R) -> R {
        f(&mut self.lock())
    }

    pub fn origins(&self) -> Vec<String> {
        self.lock().origins.clone()
    }

    pub fn api_origin(&self) -> Option<String> {
        self.lock().api_origin.clone()
    }

    pub fn set_origins(&self, origins: Vec<String>) {
        self.lock().origins = origins;
    }

    pub fn bounds(&self) -> Bounds {
        self.lock().bounds
    }
}

fn emit_state(app: &AppHandle) {
    let state = app.state::<Arc<Workspace>>().snapshot();
    let _ = app.emit("workspace-updated", state);
}

fn apply_visibility(app: &AppHandle, manager: &TabManager) {
    let focused = manager.focused.clone();
    let visible = manager.visible;
    let bounds = manager.bounds;
    for tab in &manager.browser {
        let Some(webview) = app.get_webview(&tab.id) else {
            continue;
        };
        let show = visible
            && !manager.page_covered
            && bounds_ready(bounds)
            && focused.as_deref() == Some(tab.id.as_str());
        if show {
            place_page(&webview, bounds);
            let _ = webview.show();
        } else {
            let _ = webview.hide();
        }
    }
}

fn place_page(webview: &Webview, bounds: Bounds) {
    #[cfg(not(target_os = "macos"))]
    let _ = webview.set_bounds(Rect {
        position: Position::Logical(LogicalPosition::new(bounds.x, bounds.y)),
        size: Size::Logical(LogicalSize::new(
            bounds.width.max(1.0),
            bounds.height.max(1.0),
        )),
    });
    align_page(webview, bounds.x, bounds.y, bounds.width, bounds.height);
}

fn attach_page(app: &AppHandle, tab: &BrowserTab, bounds: Bounds) -> Result<(), String> {
    if app.get_webview(&tab.id).is_some() {
        return Ok(());
    }
    let window = app
        .get_window("main")
        .ok_or_else(|| "The main window is missing.".to_string())?;
    let url = Url::parse(&tab.url).map_err(|err| err.to_string())?;
    let label = tab.id.clone();
    let nav_id = label.clone();
    let handle = app.clone();
    let nav_handle = app.clone();
    let title_handle = app.clone();
    let window_handle = app.clone();
    let script = bridge_script();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(url))
        .initialization_script(script)
        .on_navigation(move |url| {
            if !navigation_allowed(url) {
                return false;
            }
            let id = nav_id.clone();
            let href = url.to_string();
            let changed = nav_handle
                .state::<Arc<Workspace>>()
                .with(|manager| manager.apply_engine(&id, Some(&href), None, None, None, None));
            if changed {
                emit_state(&nav_handle);
            }
            true
        })
        .on_document_title_changed(move |webview, title| {
            let id = webview.label().to_string();
            // The page address comes from navigation. Reading the webview URL
            // here aborts while WebKit still has no address.
            let changed = title_handle
                .state::<Arc<Workspace>>()
                .with(|manager| manager.apply_engine(&id, None, Some(&title), None, None, None));
            if changed {
                emit_state(&title_handle);
            }
        })
        .on_new_window(move |url, _features| {
            if !navigation_allowed(&url) || url.scheme() == "about" {
                return NewWindowResponse::Deny;
            }
            let href = url.to_string();
            let app = window_handle.clone();
            let _ = window_handle.run_on_main_thread(move || {
                let _ = create_tab(&app, &href, None);
            });
            NewWindowResponse::Deny
        })
        .on_page_load(move |webview, payload| {
            if !is_page_webview(webview.label()) {
                return;
            }
            let id = webview.label().to_string();
            let url = payload.url().to_string();
            let loading = payload.event() != PageLoadEvent::Finished;
            if payload.event() == PageLoadEvent::Finished {
                let _ = webview.eval(bridge_script());
            }
            let changed = handle.state::<Arc<Workspace>>().with(|manager| {
                manager.apply_engine(&id, Some(&url), None, Some(loading), None, None)
            });
            if changed {
                emit_state(&handle);
            }
            pull_history(&handle, &webview, &id);
        });
    window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map_err(|err| format!("Could not open the page: {err}"))?;
    if let Some(webview) = app.get_webview(&tab.id) {
        enable_gestures(&webview);
        pull_history(app, &webview, &tab.id);
    }
    Ok(())
}

fn pull_history(app: &AppHandle, webview: &Webview, tab_id: &str) {
    let handle = app.clone();
    let id = tab_id.to_string();
    read_history_buttons(webview, move |back, forward| {
        let changed = handle
            .state::<Arc<Workspace>>()
            .with(|manager| manager.apply_engine(&id, None, None, None, Some(back), Some(forward)));
        if changed {
            emit_state(&handle);
        }
    });
}

enum OpenAction {
    Focus(String),
    Create(BrowserTab, Bounds),
}

fn create_tab(
    app: &AppHandle,
    url: &str,
    instance_id: Option<String>,
) -> Result<BrowserTab, String> {
    let workspace = app.state::<Arc<Workspace>>();
    let action = workspace.with(|manager| {
        manager.visible = true;
        if let Some(instance) = instance_id.as_deref() {
            if let Some(existing) = manager
                .browser
                .iter()
                .find(|tab| tab.instance_id.as_deref() == Some(instance))
            {
                return Ok(OpenAction::Focus(existing.id.clone()));
            }
        }
        let tab = manager.open(url, instance_id)?;
        Ok::<_, String>(OpenAction::Create(tab, manager.bounds))
    })?;
    match action {
        OpenAction::Focus(id) => {
            workspace_focus_tab(app.clone(), id.clone())?;
            workspace
                .with(|manager| manager.browser.iter().find(|tab| tab.id == id).cloned())
                .ok_or_else(|| "That tab is not open.".to_string())
        }
        OpenAction::Create(tab, bounds) => {
            if let Err(err) = attach_page(app, &tab, bounds) {
                workspace.with(|manager| {
                    let _ = manager.discard(&tab.id);
                });
                return Err(err);
            }
            workspace.with(|manager| {
                apply_visibility(app, manager);
            });
            emit_state(app);
            Ok(tab)
        }
    }
}

#[tauri::command]
pub fn workspace_state(workspace: State<'_, Arc<Workspace>>) -> WorkspaceState {
    workspace.snapshot()
}

#[tauri::command]
pub fn workspace_new_tab(app: AppHandle) -> Result<WorkspaceState, String> {
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.visible = true;
        manager.open_blank();
        manager.bind_focused_agent();
        apply_visibility(&app, manager);
    });
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_open_tab(app: AppHandle, url: String) -> Result<WorkspaceState, String> {
    create_tab(&app, &url, None)?;
    Ok(app.state::<Arc<Workspace>>().snapshot())
}

#[tauri::command]
pub fn workspace_open_instance(
    app: AppHandle,
    shell: State<'_, crate::Shell>,
    instance_id: String,
) -> Result<WorkspaceState, String> {
    let view = shell.catalog()?;
    let instance = view
        .instances
        .iter()
        .find(|item| item.id == instance_id)
        .ok_or_else(|| "That app is not in the catalog.".to_string())?;
    if instance.status != crate::catalog::InstanceStatus::Running {
        return Err("Set up this app first.".into());
    }
    let url = instance
        .app_url
        .clone()
        .ok_or_else(|| "This app has no address yet.".to_string())?;
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.instance_id = instance_id.clone();
        manager.visible = true;
        manager.origins = crate::http_gate::origins_from_instance(
            instance.app_url.as_deref(),
            instance.api_url.as_deref(),
        );
        manager.api_origin = instance.api_url.clone().filter(|value| !value.is_empty());
        ensure_agents(&app, manager);
    });
    create_tab(&app, &url, Some(instance_id))?;
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_focus_tab(app: AppHandle, id: String) -> Result<WorkspaceState, String> {
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.focus(&id)?;
        manager.bind_focused_agent();
        manager.visible = true;
        apply_visibility(&app, manager);
        Ok::<_, String>(())
    })?;
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_close_tab(app: AppHandle, id: String) -> Result<WorkspaceState, String> {
    let workspace = app.state::<Arc<Workspace>>();
    let closed = workspace.with(|manager| manager.close(&id))?;
    if let Some(webview) = app.get_webview(&closed.id) {
        let _ = webview.close();
    }
    workspace.with(|manager| apply_visibility(&app, manager));
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_set_page_covered(app: AppHandle, covered: bool) -> Result<(), String> {
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.page_covered = covered;
        apply_visibility(&app, manager);
    });
    Ok(())
}

#[tauri::command]
pub fn workspace_set_bounds(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if ![x, y, width, height].iter().all(|value| value.is_finite()) {
        return Ok(());
    }
    if width < 48.0 || height < 48.0 {
        return Ok(());
    }
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.bounds = Bounds {
            x,
            y,
            width,
            height,
        };
        apply_visibility(&app, manager);
    });
    Ok(())
}

#[tauri::command]
pub fn workspace_navigate(app: AppHandle, url: String) -> Result<WorkspaceState, String> {
    let url = typed_address(&url)?;
    let workspace = app.state::<Arc<Workspace>>();
    let id = workspace
        .with(|manager| manager.focused.clone())
        .ok_or_else(|| "Open a page first.".to_string())?;
    if app.get_webview(&id).is_none() {
        workspace.with(|manager| {
            manager.apply_engine(&id, Some(&url), None, Some(true), None, None);
        });
        let (tab, bounds) = workspace.with(|manager| {
            (
                manager.browser.iter().find(|tab| tab.id == id).cloned(),
                manager.bounds,
            )
        });
        let tab = tab.ok_or_else(|| "That tab is not open.".to_string())?;
        attach_page(&app, &tab, bounds)?;
        workspace.with(|manager| apply_visibility(&app, manager));
        emit_state(&app);
        return Ok(workspace.snapshot());
    }
    let parsed = Url::parse(&url).map_err(|err| err.to_string())?;
    let webview = app
        .get_webview(&id)
        .ok_or_else(|| "Open a page first.".to_string())?;
    webview
        .navigate(parsed)
        .map_err(|err| format!("Could not open the page: {err}"))?;
    workspace.with(|manager| {
        manager.apply_engine(&id, Some(&url), None, Some(true), None, None);
    });
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_history(app: AppHandle, direction: String) -> Result<(), String> {
    let action = parse_history_action(&direction)?;
    let workspace = app.state::<Arc<Workspace>>();
    let (id, loading) = workspace
        .with(|manager| manager.focused().map(|tab| (tab.id.clone(), tab.loading)))
        .ok_or_else(|| "Open a page first.".to_string())?;
    let webview = app
        .get_webview(&id)
        .ok_or_else(|| "Open a page first.".to_string())?;
    let action = if action == HistoryAction::Reload && loading {
        HistoryAction::Stop
    } else {
        action
    };
    run_history(&webview, action)?;
    if action == HistoryAction::Reload {
        workspace.with(|manager| {
            manager.apply_engine(&id, None, None, Some(true), None, None);
        });
        emit_state(&app);
    }
    if matches!(action, HistoryAction::Stop) {
        workspace.with(|manager| {
            manager.apply_engine(&id, None, None, Some(false), None, None);
        });
        emit_state(&app);
    }
    pull_history(&app, &webview, &id);
    Ok(())
}

#[tauri::command]
pub fn workspace_new_agent(app: AppHandle) -> Result<WorkspaceState, String> {
    let workspace = app.state::<Arc<Workspace>>();
    let tab = workspace.with(|manager| manager.new_agent());
    if let Some(shell) = app.try_state::<crate::Shell>() {
        let _ = crate::chats::create(
            &crate::chats::dir(shell.data_dir()),
            &tab.id,
            &tab.session_id,
        );
    }
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_close_agent(app: AppHandle, id: String) -> Result<WorkspaceState, String> {
    let workspace = app.state::<Arc<Workspace>>();
    let tab = workspace.with(|manager| manager.close_agent(&id))?;
    if let Some(shell) = app.try_state::<crate::Shell>() {
        let _ =
            crate::chats::set_open(&crate::chats::dir(shell.data_dir()), &tab.session_id, false);
    }
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_open_chat(app: AppHandle, session_id: String) -> Result<WorkspaceState, String> {
    let folder = {
        let shell = app
            .try_state::<crate::Shell>()
            .ok_or_else(|| "The app host is not ready.".to_string())?;
        crate::chats::dir(shell.data_dir())
    };
    let record = crate::chats::load(&folder, &session_id)?;
    crate::chats::set_open(&folder, &session_id, true)?;
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.visible = true;
        manager.open_saved(&record);
        manager.bind_focused_agent();
    });
    emit_state(&app);
    Ok(workspace.snapshot())
}

fn ensure_agents(app: &AppHandle, manager: &mut TabManager) {
    let records = app
        .try_state::<crate::Shell>()
        .map(|shell| {
            crate::chats::load_all(&crate::chats::dir(shell.data_dir())).unwrap_or_default()
        })
        .unwrap_or_default();
    if manager.agents.is_empty() {
        manager.restore_open(&records);
    }
    if manager.agents.is_empty() && records.is_empty() {
        let tab = manager.new_agent();
        if let Some(shell) = app.try_state::<crate::Shell>() {
            let _ = crate::chats::create(
                &crate::chats::dir(shell.data_dir()),
                &tab.id,
                &tab.session_id,
            );
        }
    }
}

pub(crate) fn note_chat_title(app: &AppHandle, session_id: &str, title: &str) {
    let Some(workspace) = app.try_state::<Arc<Workspace>>() else {
        return;
    };
    if workspace.with(|manager| manager.rename_session(session_id, title)) {
        emit_state(app);
    }
}

#[tauri::command]
pub fn workspace_focus_agent(app: AppHandle, id: String) -> Result<WorkspaceState, String> {
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.focus_agent(&id)?;
        manager.bind_focused_agent();
        Ok::<_, String>(())
    })?;
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_show(app: AppHandle) -> Result<WorkspaceState, String> {
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.visible = true;
        ensure_agents(&app, manager);
        if manager.browser.is_empty() {
            manager.open_blank();
        }
        manager.bind_focused_agent();
        apply_visibility(&app, manager);
    });
    emit_state(&app);
    Ok(workspace.snapshot())
}

#[tauri::command]
pub fn workspace_hide(app: AppHandle) -> Result<(), String> {
    let workspace = app.state::<Arc<Workspace>>();
    workspace.with(|manager| {
        manager.visible = false;
        apply_visibility(&app, manager);
    });
    Ok(())
}

pub fn start_offer_poll(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(300));
            let tabs: Vec<String> = app
                .try_state::<Arc<Workspace>>()
                .map(|workspace| {
                    workspace.with(|manager| {
                        if !manager.visible {
                            Vec::new()
                        } else {
                            manager.browser.iter().map(|tab| tab.id.clone()).collect()
                        }
                    })
                })
                .unwrap_or_default();
            for id in tabs {
                let Some(webview) = app.get_webview(&id) else {
                    continue;
                };
                pull_history(&app, &webview, &id);
                let handle = app.clone();
                let tab_id = id.clone();
                let script = r#"(function(){try{var o=window.__buhoOffer;if(!o||!o.mcp||!o.mcp.url)return null;return {name:String(o.mcp.name||'app'),url:String(o.mcp.url)};}catch(e){return null;}})()"#;
                let _ = webview.eval_with_callback(script, move |raw| {
                    let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                    let offer = parse_offer(&value);
                    let mut changed = false;
                    let mut previous: Option<McpOffer> = None;
                    handle.state::<Arc<Workspace>>().with(|manager| {
                        previous = manager.offer(&tab_id).cloned();
                        changed = match (&previous, &offer) {
                            (Some(left), Some(right)) => left != right,
                            (None, None) => false,
                            _ => true,
                        };
                        manager.set_offer(&tab_id, offer.clone());
                    });
                    if !changed {
                        return;
                    }
                    if let Some(harness) = handle.try_state::<Arc<crate::harness::Harness>>() {
                        let harness = (*harness).clone();
                        if let Some(offer) = offer {
                            let origins = handle.state::<Arc<Workspace>>().origins();
                            let allowed = url::Url::parse(&offer.url)
                                .map(|parsed| crate::http_gate::allowed(&parsed, &origins))
                                .unwrap_or(false);
                            if allowed {
                                harness.connect_mcp(&tab_id, &offer.name, &offer.url);
                            } else if let Some(gate) =
                                handle.try_state::<Arc<crate::permissions::PermissionGate>>()
                            {
                                let key = format!("{}:{}", tab_id, offer.url);
                                let already = handle.state::<Arc<Workspace>>().with(|manager| {
                                    manager.pending_mcp.contains_key(&tab_id)
                                });
                                if already {
                                    return;
                                }
                                handle.state::<Arc<Workspace>>().with(|manager| {
                                    manager
                                        .pending_mcp
                                        .insert(tab_id.clone(), key);
                                });
                                let session = handle.state::<Arc<Workspace>>().with(|manager| {
                                    manager
                                        .focused_agent_session()
                                        .unwrap_or_else(|| "session-1".into())
                                });
                                let gate = (*gate).clone();
                                let args = json!({ "url": offer.url, "name": offer.name, "tabId": tab_id });
                                let tab = tab_id.clone();
                                let name = offer.name.clone();
                                let url = offer.url.clone();
                                let prompt = handle.clone();
                                std::thread::spawn(move || {
                                    let allow = gate.ask(&prompt, &session, "mcp.connect", &args);
                                    prompt.state::<Arc<Workspace>>().with(|manager| {
                                        manager.pending_mcp.remove(&tab);
                                    });
                                    if allow {
                                        harness.connect_mcp(&tab, &name, &url);
                                    }
                                });
                            }
                        } else if let Some(previous) = previous {
                            harness.disconnect_mcp(&tab_id, &previous.name);
                        }
                    }
                });
            }
        }
    });
}

fn parse_offer(value: &Value) -> Option<McpOffer> {
    if value.is_null() {
        return None;
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("app")
        .trim()
        .to_string();
    let url = public_http_url(value.get("url").and_then(Value::as_str)?.trim()).ok()?;
    Some(McpOffer {
        name: if name.is_empty() { "app".into() } else { name },
        url,
    })
}

pub fn reload_tab(app: &AppHandle, tab_id: &str) -> Result<bool, String> {
    let open = app.try_state::<Arc<Workspace>>().is_some_and(|workspace| {
        workspace.with(|manager| {
            manager
                .list()
                .iter()
                .any(|tab| tab.id == tab_id && !tab.url.trim().is_empty())
        })
    });
    if !open {
        return Ok(false);
    }
    let Some(webview) = app.get_webview(tab_id) else {
        return Ok(false);
    };
    run_history(&webview, HistoryAction::Reload)?;
    if let Some(workspace) = app.try_state::<Arc<Workspace>>() {
        workspace.with(|manager| {
            manager.apply_engine(tab_id, None, None, Some(true), None, None);
        });
        emit_state(app);
    }
    pull_history(app, &webview, tab_id);
    Ok(true)
}

pub fn navigate_tab(app: &AppHandle, tab_id: &str, url: &str) -> Result<Value, String> {
    let url = public_http_url(url)?;
    let parsed = Url::parse(&url).map_err(|err| err.to_string())?;
    let webview = app
        .get_webview(tab_id)
        .ok_or_else(|| "Open a page first.".to_string())?;
    webview
        .navigate(parsed)
        .map_err(|err| format!("Could not open the page: {err}"))?;
    app.state::<Arc<Workspace>>()
        .with(|manager| manager.set_url(tab_id, &url, None));
    emit_state(app);
    Ok(json!({ "url": url, "tabId": tab_id }))
}

pub fn open_tab(app: &AppHandle, url: &str) -> Result<Value, String> {
    let tab = create_tab(app, url, None)?;
    Ok(json!({ "id": tab.id, "url": tab.url }))
}

pub fn list_tabs(app: &AppHandle) -> Value {
    let state = app.state::<Arc<Workspace>>().snapshot();
    json!({
        "tabs": state.browser_tabs,
        "activeId": state.active_browser_id
    })
}

pub fn focus_tab(app: &AppHandle, id: &str) -> Result<Value, String> {
    workspace_focus_tab(app.clone(), id.to_string())?;
    Ok(json!({ "id": id }))
}

pub fn close_tab(app: &AppHandle, id: &str) -> Result<Value, String> {
    workspace_close_tab(app.clone(), id.to_string())?;
    Ok(json!({ "id": id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_urls() {
        let mut manager = TabManager::default();
        assert!(manager.open("javascript:alert(1)", None).is_err());
        assert!(manager.open("file:///tmp/x", None).is_err());
        assert!(manager.open("not a url", None).is_err());
        assert!(public_http_url("https://user:secret@example.com").is_err());
    }

    #[test]
    fn labels_pages_in_order() {
        let mut manager = TabManager::default();
        let first = manager.open("https://example.com", None).unwrap();
        let second = manager.open("https://example.org/path", None).unwrap();
        assert_eq!(first.id, "page-1");
        assert_eq!(second.id, "page-2");
        assert!(is_page_webview(&first.id));
        assert_eq!(manager.list().len(), 2);
        assert_eq!(manager.focused().unwrap().id, "page-2");
        manager.focus("page-1").unwrap();
        assert_eq!(manager.focused().unwrap().id, "page-1");
        manager.close("page-1").unwrap();
        assert_eq!(manager.focused().unwrap().id, "page-2");
        assert_eq!(manager.list().len(), 1);
    }

    #[test]
    fn agents_bind_to_the_focused_page() {
        let mut manager = TabManager::default();
        manager
            .open("https://example.com", Some("fourss-default".into()))
            .unwrap();
        let agent = manager.new_agent();
        assert_eq!(agent.bound_tab_ids, vec!["page-1".to_string()]);
        assert_eq!(
            manager.default_tab_id(&agent.session_id).as_deref(),
            Some("page-1")
        );
        assert_eq!(
            manager.focused_agent_session().as_deref(),
            Some("session-1")
        );
    }

    #[test]
    fn an_agent_chat_can_close_and_stay_out_of_the_strip() {
        let mut manager = TabManager::default();
        let first = manager.new_agent();
        let second = manager.new_agent();
        manager.close_agent(&second.id).unwrap();
        assert_eq!(manager.focused_agent.as_deref(), Some(first.id.as_str()));
        manager.close_agent(&first.id).unwrap();
        assert!(manager.agents.is_empty());
        assert!(manager.focused_agent.is_none());
    }

    #[test]
    fn the_last_tab_stays_open() {
        let mut manager = TabManager::default();
        manager.open("https://example.com", None).unwrap();
        assert!(manager.close("page-1").is_err());
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.focused().unwrap().id, "page-1");
    }

    #[test]
    fn closing_a_middle_tab_focuses_its_neighbor() {
        let mut manager = TabManager::default();
        manager.open("https://example.com/a", None).unwrap();
        manager.open("https://example.com/b", None).unwrap();
        manager.open("https://example.com/c", None).unwrap();
        manager.focus("page-2").unwrap();
        manager.close("page-2").unwrap();
        assert_eq!(manager.focused().unwrap().id, "page-3");
        assert_eq!(manager.list().len(), 2);
    }

    #[test]
    fn page_bounds_need_a_real_content_hole() {
        assert!(!bounds_ready(Bounds::default()));
        assert!(bounds_ready(Bounds {
            x: 10.0,
            y: 80.0,
            width: 400.0,
            height: 300.0,
        }));
        assert!(!bounds_ready(Bounds {
            x: 0.0,
            y: 0.0,
            width: f64::NAN,
            height: 200.0,
        }));
    }

    #[test]
    fn a_blank_tab_waits_for_an_address() {
        let mut manager = TabManager::default();
        let tab = manager.open_blank();
        assert_eq!(tab.title, "New tab");
        assert!(tab.url.is_empty());
        assert_eq!(manager.focused().unwrap().id, tab.id);
    }

    #[test]
    fn open_accepts_a_host_without_a_scheme() {
        let mut manager = TabManager::default();
        let tab = manager.open("example.com/app", None).unwrap();
        assert_eq!(tab.url, "https://example.com/app");
    }

    #[test]
    fn engine_state_tracks_the_webview() {
        let mut manager = TabManager::default();
        manager.open("https://example.com/app", None).unwrap();
        assert!(manager.list()[0].loading);
        assert!(!manager.list()[0].can_go_back);
        assert!(manager.apply_engine(
            "page-1",
            Some("https://example.com/app/settings"),
            Some("Settings"),
            Some(false),
            Some(true),
            Some(false),
        ));
        let tab = manager.focused().unwrap();
        assert_eq!(tab.url, "https://example.com/app/settings");
        assert_eq!(tab.title, "Settings");
        assert!(!tab.loading);
        assert!(tab.can_go_back);
        assert!(!tab.can_go_forward);
        assert!(!manager.apply_engine("page-1", Some("about:blank"), Some(""), None, None, None));
        assert_eq!(
            manager.focused().unwrap().url,
            "https://example.com/app/settings"
        );
        assert_eq!(manager.focused().unwrap().title, "Settings");
    }

    #[test]
    fn offers_must_be_http() {
        assert!(parse_offer(&json!({"name":"fourss","url":"javascript:alert(1)"})).is_none());
        assert!(parse_offer(&json!({"name":"fourss","url":"file:///tmp"})).is_none());
        let offer = parse_offer(&json!({
            "name": "fourss",
            "url": "http://127.0.0.1:3012/__agent-helper/mcp?token=secret"
        }))
        .unwrap();
        assert_eq!(offer.name, "fourss");
        assert!(offer.url.starts_with("http://127.0.0.1:3012/"));
    }
}
