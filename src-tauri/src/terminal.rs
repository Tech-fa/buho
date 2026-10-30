use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use base64::Engine;
use portable_pty::{Child, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter};

use crate::agent_context::{AgentContext, AgentPlace};

/// Webview label for the terminal docked in the preview. It is not a window label:
/// a window with this label would grant shell commands to every webview in it.
pub const TERMINAL_WEBVIEW: &str = "terminal";

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send>,
    reader: Option<JoinHandle<()>>,
}

pub struct TerminalHost {
    session: Mutex<Option<PtySession>>,
    generation: Arc<AtomicU64>,
    context: AgentContext,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub shell: String,
    pub api_url: Option<String>,
    pub mcp_url: Option<String>,
}

impl TerminalHost {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        let context = AgentContext::open(data_dir);
        if let Some(home) = home_dir() {
            if let Err(err) = context.repair(&home) {
                crate::journal::write("terminal", &err);
            }
        }
        Self {
            session: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
            context,
        }
    }

    pub fn start(
        &self,
        app: AppHandle,
        rows: u16,
        cols: u16,
        extra_env: Vec<(String, String)>,
    ) -> Result<TerminalInfo, String> {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut slot = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = None;

        let shell = user_shell();
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| format!("Could not open a terminal: {err}"))?;

        let mut command = CommandBuilder::new(&shell);
        if cfg!(unix) {
            command.arg("-l");
        }
        if let Some(home) = home_dir() {
            command.cwd(home);
        }
        for (key, value) in &extra_env {
            command.env(key, value);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|err| format!("Could not start {shell}: {err}"))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|err| format!("Could not read the terminal: {err}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|err| format!("Could not write to the terminal: {err}"))?;

        if let Some(home) = home_dir() {
            if let Err(err) = self
                .context
                .publish(&home, generation, &place_from_env(&extra_env))
            {
                crate::journal::write("terminal", &err);
            }
        }

        let thread_app = app.clone();
        let current = Arc::clone(&self.generation);
        let context = self.context.clone();
        let reader_thread = std::thread::spawn(move || {
            read_terminal(reader, thread_app, generation, current);
            context.clear_generation(generation);
        });
        *slot = Some(PtySession {
            master: pair.master,
            writer,
            child,
            reader: Some(reader_thread),
        });
        Ok(info_from_env(&extra_env))
    }

    pub fn write(&self, data: &str) -> Result<(), String> {
        let mut slot = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = slot.as_mut() else {
            return Err("The terminal is not running.".into());
        };
        session
            .writer
            .write_all(data.as_bytes())
            .and_then(|_| session.writer.flush())
            .map_err(|err| format!("Could not write to the terminal: {err}"))
    }

    pub fn note_app_url(&self, url: &str) {
        if let Err(err) = self.context.note_app_url(url) {
            crate::journal::write("terminal", &err);
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        let slot = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = slot.as_ref() else {
            return Ok(());
        };
        session
            .master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| format!("Could not resize the terminal: {err}"))
    }
}

impl Drop for TerminalHost {
    fn drop(&mut self) {
        let mut slot = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = None;
        self.context.clear_all();
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _thread = self.reader.take();
    }
}

pub fn agent_env(values: &Map<String, Value>) -> Vec<(String, String)> {
    let mut env = vec![("TERM".into(), "xterm-256color".into())];
    if let Some(url) = string_setting(values, "apiUrl") {
        env.push(("API_URL".into(), url));
    }
    env
}

fn place_from_env(extra_env: &[(String, String)]) -> AgentPlace {
    AgentPlace {
        app_url: env_value(extra_env, "APP_URL"),
        mcp_url: env_value(extra_env, "APP_MCP_URL"),
        api_url: env_value(extra_env, "API_URL"),
    }
}

fn env_value(extra_env: &[(String, String)], key: &str) -> String {
    extra_env
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn info_from_env(extra_env: &[(String, String)]) -> TerminalInfo {
    TerminalInfo {
        shell: user_shell(),
        api_url: extra_env
            .iter()
            .find(|(key, _)| key == "API_URL")
            .map(|(_, value)| value.clone()),
        mcp_url: extra_env
            .iter()
            .find(|(key, _)| key == "APP_MCP_URL")
            .map(|(_, value)| value.clone()),
    }
}

fn string_setting(values: &Map<String, Value>, key: &str) -> Option<String> {
    let text = values.get(key)?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn user_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
    }
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

fn read_terminal(
    mut reader: Box<dyn Read + Send>,
    app: AppHandle,
    generation: u64,
    current: Arc<AtomicU64>,
) {
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(size) => {
                if current.load(Ordering::Relaxed) != generation {
                    break;
                }
                let payload = base64::engine::general_purpose::STANDARD.encode(&buffer[..size]);
                let _ = app.emit_to(TERMINAL_WEBVIEW, "terminal-output", payload);
            }
        }
    }
    if current.load(Ordering::Relaxed) == generation {
        let _ = app.emit_to(
            TERMINAL_WEBVIEW,
            "terminal-exit",
            "The shell exited. Pull the terminal down, then pull it up to start again.",
        );
    }
}

pub fn terminal_page(setup_url: &str) -> Result<url::Url, String> {
    let mut page = url::Url::parse(setup_url).map_err(|err| err.to_string())?;
    page.set_path("terminal.html");
    page.set_query(None);
    page.set_fragment(None);
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::{agent_env, terminal_page};
    use serde_json::json;

    #[test]
    fn api_url_is_passed_to_the_shell_when_the_bundle_saved_one() {
        let values = json!({"apiUrl": "http://127.0.0.1:3002", "note": ""})
            .as_object()
            .unwrap()
            .clone();
        let env = agent_env(&values);
        assert!(
            env.iter()
                .any(|(key, value)| key == "TERM" && value == "xterm-256color")
        );
        assert!(
            env.iter()
                .any(|(key, value)| key == "API_URL" && value == "http://127.0.0.1:3002")
        );
    }

    #[test]
    fn a_blank_api_url_is_left_unset() {
        let values = json!({"apiUrl": "  "}).as_object().unwrap().clone();
        let env = agent_env(&values);
        assert!(!env.iter().any(|(key, _)| key == "API_URL"));
    }

    #[test]
    fn terminal_page_replaces_the_setup_path() {
        let page = terminal_page("http://localhost:1420/index.html?x=1#top").unwrap();
        assert_eq!(page.as_str(), "http://localhost:1420/terminal.html");
    }
}
