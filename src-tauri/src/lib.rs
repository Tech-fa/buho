mod agent_context;
mod browser;
mod control;
mod deploy;
mod journal;
mod server;
mod settings;
mod terminal;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use control::{BridgeTarget, PageExecutor, public_app_url, remembered_app_url, serve_control};
use serde::Serialize;
use serde_json::Value;
use server::{ServerHandle, Session, serve_local};
use settings::{
    APP_NAME, AppView, LoadedUi, empty_view, load_ui, prepare_ui_dir, remember_ui_path,
    remembered_ui_path, save_values, view,
};
use tauri::menu::{Menu, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use terminal::{TerminalHost, TerminalInfo, agent_env};

use deploy::{DeploymentInput, DeploymentView};

struct Shell {
    shared: Arc<Mutex<Session>>,
    server: ServerHandle,
    data_dir: PathBuf,
    setup_url: String,
    app_url: Mutex<String>,
    mcp_url: Mutex<String>,
    mcp_token: Mutex<String>,
    deploy_status: Arc<Mutex<String>>,
    deploy_running: Arc<AtomicBool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShellState {
    #[serde(flatten)]
    view: AppView,
    server_url: String,
    data_dir: String,
    app_url: String,
    mcp_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveOutcome {
    state: ShellState,
    field_errors: std::collections::BTreeMap<String, String>,
    message: String,
}

fn requested_ui() -> Option<(PathBuf, String)> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--ui" {
            return args
                .next()
                .map(|path| (PathBuf::from(path), "argument".into()));
        }
        if let Some(path) = arg.strip_prefix("--ui=") {
            return Some((PathBuf::from(path), "argument".into()));
        }
    }
    std::env::var_os("APP_SHELL_UI").map(|path| (PathBuf::from(path), "environment".into()))
}

impl Shell {
    fn start(
        data_dir: PathBuf,
        setup_url: String,
        requested: Option<(PathBuf, String)>,
    ) -> Result<Shell, String> {
        std::fs::create_dir_all(&data_dir)
            .map_err(|err| format!("Could not create the app data folder: {err}"))?;
        deploy::ensure_config(&data_dir)?;
        let (ui_dir, ui_source, loaded, status) = select_ui(&data_dir, requested)?;
        let shared = Arc::new(Mutex::new(Session {
            ui_dir,
            ui_source,
            loaded,
            status,
        }));
        let server = serve_local(Arc::clone(&shared))?;
        Ok(Shell {
            shared,
            server,
            app_url: Mutex::new(remembered_app_url(&data_dir).unwrap_or_default()),
            data_dir,
            setup_url,
            mcp_url: Mutex::new(String::new()),
            mcp_token: Mutex::new(String::new()),
            deploy_status: Arc::new(Mutex::new(String::new())),
            deploy_running: Arc::new(AtomicBool::new(false)),
        })
    }

    fn deployment_view(&self) -> Result<DeploymentView, String> {
        let config = deploy::ensure_config(&self.data_dir)?;
        let status = self
            .deploy_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        Ok(deploy::view(
            config,
            &status,
            self.deploy_running.load(Ordering::SeqCst),
        ))
    }

    fn session_state(&self, session: &Session) -> ShellState {
        let mut state = state_from(session, &self.server.url, &self.data_dir);
        state.app_url = self
            .app_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        state.mcp_url = self
            .mcp_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        state
    }

    fn set_mcp_url(&self, url: String) {
        *self
            .mcp_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = url;
    }

    fn set_mcp_token(&self, token: String) {
        *self
            .mcp_token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = token;
    }

    fn app_url(&self) -> String {
        self.app_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn remember_opened_url(&self, url: &str) -> Result<(), String> {
        control::remember_app_url(&self.data_dir, url)?;
        *self
            .app_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = url.to_string();
        Ok(())
    }

    fn snapshot(&self) -> ShellState {
        let session = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.session_state(&session)
    }

    fn set_ui_path(&self, path: PathBuf) -> Result<ShellState, String> {
        let (ui_dir, loaded) = try_load(&self.data_dir, &path)?;
        remember_ui_path(&self.data_dir, &ui_dir)?;
        let mut session = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        session.ui_dir = Some(ui_dir);
        session.ui_source = "saved".into();
        session.loaded = Some(loaded);
        session.status.clear();
        Ok(self.session_state(&session))
    }

    fn save(&self, values: Value) -> SaveOutcome {
        let mut session = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = match session.loaded.as_mut() {
            Some(loaded) => save_values(&self.data_dir, loaded, &values),
            None => Err(std::collections::BTreeMap::from([(
                String::new(),
                "Choose a web app bundle first.".into(),
            )])),
        };
        match saved {
            Ok(()) => SaveOutcome {
                state: self.session_state(&session),
                field_errors: std::collections::BTreeMap::new(),
                message: String::new(),
            },
            Err(errors) => {
                let message = errors.get("").cloned().unwrap_or_default();
                let field_errors = errors
                    .into_iter()
                    .filter(|(key, _)| !key.is_empty())
                    .collect();
                SaveOutcome {
                    state: self.session_state(&session),
                    field_errors,
                    message,
                }
            }
        }
    }
}

fn state_from(session: &Session, server_url: &str, data_dir: &Path) -> ShellState {
    let view = match (session.ui_dir.as_ref(), session.loaded.as_ref()) {
        (Some(ui_dir), Some(loaded)) => view(ui_dir, &session.ui_source, loaded, &session.status),
        _ => empty_view(&session.status),
    };
    ShellState {
        view,
        server_url: server_url.into(),
        data_dir: data_dir.display().to_string(),
        app_url: String::new(),
        mcp_url: String::new(),
    }
}

fn select_ui(
    data_dir: &Path,
    requested: Option<(PathBuf, String)>,
) -> Result<(Option<PathBuf>, String, Option<LoadedUi>, String), String> {
    let mut errors = Vec::new();
    if let Some((path, source)) = requested {
        match try_load(data_dir, &path) {
            Ok((ui_dir, loaded)) => {
                let _ = remember_ui_path(data_dir, &ui_dir);
                return Ok((Some(ui_dir), source, Some(loaded), String::new()));
            }
            Err(err) => errors.push(err),
        }
    }
    if let Some(path) = remembered_ui_path(data_dir) {
        match try_load(data_dir, &path) {
            Ok((ui_dir, loaded)) => {
                return Ok((Some(ui_dir), "saved".into(), Some(loaded), String::new()));
            }
            Err(err) => errors.push(format!("Saved web app bundle: {err}")),
        }
    }
    Ok((None, "none".into(), None, errors.join(" ")))
}

fn try_load(data_dir: &Path, path: &Path) -> Result<(PathBuf, LoadedUi), String> {
    let ui_dir = prepare_ui_dir(path)?;
    let loaded = load_ui(data_dir, &ui_dir)?;
    Ok((ui_dir, loaded))
}

fn agent_environment(shell: &Shell) -> Vec<(String, String)> {
    let mut env = {
        let session = shell
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        session
            .loaded
            .as_ref()
            .map(|loaded| agent_env(&loaded.stored))
            .unwrap_or_else(|| agent_env(&serde_json::Map::new()))
    };
    let app_url = shell.app_url();
    let mcp_url = shell
        .mcp_url
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if !app_url.is_empty() {
        env.push(("APP_URL".into(), app_url));
    }
    let mcp_token = shell
        .mcp_token
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if !mcp_url.is_empty() {
        let authed = if mcp_token.is_empty() {
            mcp_url
        } else {
            format!("{mcp_url}?token={mcp_token}")
        };
        let tools = authed.replacen("/mcp?", "/tools.json?", 1);
        env.push(("APP_MCP_URL".into(), authed));
        env.push(("APP_TOOLS_URL".into(), tools));
    }
    env
}

fn install_menu(app: &AppHandle) -> tauri::Result<()> {
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    app.set_menu(Menu::with_items(app, &[&edit_menu])?)?;
    Ok(())
}

#[tauri::command]
fn shell_state(shell: tauri::State<Shell>) -> ShellState {
    shell.snapshot()
}

#[tauri::command]
fn available_versions() -> Result<deploy::ReleaseVersions, String> {
    journal::logged("newest versions", deploy::available_versions())
}

#[tauri::command]
fn deployment_state(shell: tauri::State<Shell>) -> Result<DeploymentView, String> {
    journal::logged("read setup", shell.deployment_view())
}

#[tauri::command]
fn save_deployment(
    shell: tauri::State<Shell>,
    input: DeploymentInput,
) -> Result<DeploymentView, String> {
    journal::logged_plain("save setup", deploy::save_input(&shell.data_dir, input))?;
    journal::logged("read setup", shell.deployment_view())
}

#[tauri::command]
fn check_token(token: String) -> Result<(), String> {
    journal::logged_plain("DigitalOcean key", deploy::verify_token(&token))
}

#[tauri::command]
fn droplet_regions(shell: tauri::State<Shell>) -> Result<Vec<deploy::RegionChoice>, String> {
    let config = journal::logged("read setup", deploy::ensure_config(&shell.data_dir))?;
    journal::logged_plain("places", deploy::list_regions(&config.digital_ocean_token))
}

#[tauri::command]
async fn open_in_app(
    app: AppHandle,
    shell: State<'_, Shell>,
    host: State<'_, TerminalHost>,
    url: String,
) -> Result<(), String> {
    let url = journal::logged("open page", public_app_url(&url))?;
    journal::logged("open page", shell.remember_opened_url(&url))?;
    let opened = journal::logged("open page", browser::show(&app, &url, &shell.setup_url));
    if opened.is_ok() {
        host.note_app_url(&url);
    }
    opened
}

#[tauri::command]
fn set_preview_terminal(
    app: AppHandle,
    height: f64,
    dragging: bool,
    snap: bool,
) -> Result<browser::DockView, String> {
    journal::logged(
        "terminal",
        browser::set_height(&app, height, dragging, snap),
    )
}

#[tauri::command]
fn toggle_preview_terminal(app: AppHandle) -> Result<browser::DockView, String> {
    journal::logged("terminal", browser::toggle(&app))
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let allowed =
        url == deploy::TOKEN_URL || url == deploy::RESEND_URL || url == deploy::BILLING_URL;
    if !allowed {
        return journal::logged(
            "open page",
            Err("That page cannot be opened from the shell.".into()),
        );
    }
    let status = std::process::Command::new("open")
        .arg(&url)
        .status()
        .map_err(|err| format!("Could not open the page: {err}"));
    journal::logged(
        "open page",
        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(_) => Err("Could not open the page.".into()),
            Err(err) => Err(err),
        },
    )
}

#[tauri::command]
fn start_deploy(app: AppHandle, shell: tauri::State<Shell>) -> Result<DeploymentView, String> {
    if shell
        .deploy_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return journal::logged(
            "make the app",
            Err("We are already working on your app.".into()),
        );
    }
    let data_dir = shell.data_dir.clone();
    let status = Arc::clone(&shell.deploy_status);
    let running = Arc::clone(&shell.deploy_running);
    {
        let mut slot = status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slot.clear();
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let mut report = |message: String| {
            let mut slot = status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = message;
            let _ = handle.emit("deploy-status", slot.clone());
        };
        let outcome = deploy::provision(&data_dir, &mut report);
        let mut slot = status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = match outcome {
            Ok(_) => "Your app is ready.".into(),
            Err(err) => {
                journal::write("make the app", &err);
                deploy::plain_failure(&err)
            }
        };
        running.store(false, Ordering::SeqCst);
    });
    journal::logged("read setup", shell.deployment_view())
}

#[tauri::command]
fn set_ui_path(
    shell: tauri::State<Shell>,
    window: WebviewWindow,
    path: String,
) -> Result<ShellState, String> {
    let state = journal::logged("web app folder", shell.set_ui_path(PathBuf::from(path)))?;
    let _ = window.set_title(APP_NAME);
    Ok(state)
}

#[tauri::command]
fn save_config(shell: tauri::State<Shell>, values: Value) -> SaveOutcome {
    let outcome = shell.save(values);
    if !outcome.message.is_empty() {
        journal::write("save settings", &outcome.message);
    }
    for (field, err) in &outcome.field_errors {
        journal::write("save settings", &format!("{field}: {err}"));
    }
    outcome
}

#[tauri::command]
fn export_configuration(shell: tauri::State<Shell>, destination: String) -> Result<(), String> {
    journal::logged(
        "save a copy",
        deploy::export_configuration(&shell.data_dir, Path::new(&destination)),
    )
}

#[tauri::command]
fn forget_resource(shell: tauri::State<Shell>, kind: String) -> Result<DeploymentView, String> {
    journal::logged(
        "remove a saved item",
        deploy::forget_resource(&shell.data_dir, &kind),
    )?;
    journal::logged("read setup", shell.deployment_view())
}

#[tauri::command]
fn delete_droplet(shell: tauri::State<Shell>) -> Result<DeploymentView, String> {
    if shell.deploy_running.load(Ordering::SeqCst) {
        return journal::logged(
            "remove the computer",
            Err("Wait until we finish making your app.".into()),
        );
    }
    journal::logged_plain(
        "remove the computer",
        deploy::delete_droplet(&shell.data_dir),
    )?;
    journal::logged("read setup", shell.deployment_view())
}

#[tauri::command]
fn terminal_start(
    app: AppHandle,
    host: State<TerminalHost>,
    shell: State<Shell>,
    rows: u16,
    cols: u16,
) -> Result<TerminalInfo, String> {
    journal::logged(
        "terminal",
        host.start(app, rows, cols, agent_environment(&shell)),
    )
}

#[tauri::command]
fn terminal_write(host: State<TerminalHost>, data: String) -> Result<(), String> {
    journal::logged("terminal", host.write(&data))
}

#[tauri::command]
fn terminal_resize(host: State<TerminalHost>, rows: u16, cols: u16) -> Result<(), String> {
    journal::logged("terminal", host.resize(rows, cols))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let bridge = Arc::new(BridgeTarget {
        setup_url: Mutex::new(String::new()),
        script: Mutex::new(String::new()),
    });
    let bridge_for_setup = Arc::clone(&bridge);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(control::bridge_plugin(bridge))
        .setup(move |app| {
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| std::io::Error::other("The main window is missing."))?;
            let setup_url = window.url().map_err(std::io::Error::other)?.to_string();
            let data_dir = app.path().app_data_dir().map_err(std::io::Error::other)?;
            journal::install(&data_dir);
            let shell = Shell::start(data_dir.clone(), setup_url.clone(), requested_ui()).map_err(
                |err| {
                    journal::write("start", &err);
                    std::io::Error::new(std::io::ErrorKind::Other, err)
                },
            )?;
            let control = serve_control(Arc::new(PageExecutor::new(
                app.handle().clone(),
                setup_url.clone(),
            )))
            .map_err(|err| {
                journal::write("start", &err);
                std::io::Error::new(std::io::ErrorKind::Other, err)
            })?;
            shell.set_mcp_url(control.mcp_url.clone());
            shell.set_mcp_token(control.token.clone());
            *bridge_for_setup
                .setup_url
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = setup_url;
            *bridge_for_setup
                .script
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                control::bridge_script().to_string();
            let _ = window.set_title(APP_NAME);
            install_menu(app.handle()).map_err(|err| {
                journal::write("start", &err.to_string());
                std::io::Error::other(err.to_string())
            })?;
            app.manage(control);
            app.manage(shell);
            app.manage(TerminalHost::new(data_dir));
            app.manage(browser::PreviewDock::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            shell_state,
            set_ui_path,
            save_config,
            terminal_start,
            terminal_write,
            terminal_resize,
            set_preview_terminal,
            toggle_preview_terminal,
            deployment_state,
            available_versions,
            save_deployment,
            check_token,
            droplet_regions,
            open_external,
            open_in_app,
            start_deploy,
            delete_droplet,
            forget_resource,
            export_configuration
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::select_ui;

    fn labels_of<'a>(value: &'a serde_json::Value, key: &str) -> Vec<&'a str> {
        value[key]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|label| label.as_str())
            .collect()
    }

    #[test]
    fn first_launch_asks_for_a_bundle() {
        let dir = std::env::temp_dir().join(format!("app-shell-first-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (ui_dir, source, loaded, status) = select_ui(&dir, None).unwrap();
        assert!(ui_dir.is_none());
        assert!(loaded.is_none());
        assert_eq!(source, "none");
        assert!(status.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_opened_page_is_not_granted_shell_commands() {
        let text = include_str!("../capabilities/default.json");
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        let windows = labels_of(&value, "windows");
        assert!(windows.contains(&"main"));
        assert!(!windows.contains(&crate::control::PAGE_WEBVIEW));
        assert!(!windows.contains(&crate::browser::PREVIEW_WINDOW));
        assert!(!windows.contains(&crate::terminal::TERMINAL_WEBVIEW));
        assert!(!windows.contains(&"*"));
        let webviews = labels_of(&value, "webviews");
        assert!(webviews.contains(&crate::terminal::TERMINAL_WEBVIEW));
        assert!(!webviews.contains(&crate::control::PAGE_WEBVIEW));
        assert!(!webviews.contains(&"*"));
    }
}
