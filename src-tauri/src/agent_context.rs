use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control::LEAVE_CONFIRMATION_TO_THE_USER;

/// Cursor's agent loads this server from `~/.cursor/mcp.json` for every project.
pub const SERVER_NAME: &str = "app-shell-preview";

const RULE_NAME: &str = "app-shell-terminal.mdc";
const RECORD_NAME: &str = "agent-terminal-context.json";
const RULE_MARKER: &str = "You are in the Buho terminal, next to the preview window.";

/// Where the preview is, and how the agent reaches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPlace {
    pub app_url: String,
    pub mcp_url: String,
    pub api_url: String,
}

#[derive(Clone)]
pub struct AgentContext {
    inner: Arc<Inner>,
}

struct Inner {
    data_dir: PathBuf,
    state: Mutex<Option<Published>>,
}

struct Published {
    generation: u64,
    place: AgentPlace,
    record: ContextRecord,
}

#[derive(Serialize, Deserialize)]
struct ContextRecord {
    mcp_path: String,
    rule_path: String,
    original_mcp_b64: Option<String>,
    original_rule_b64: Option<String>,
    replaced_server: Option<Value>,
    mcp_updated: bool,
    created_cursor: bool,
    created_rules: bool,
}

enum McpFile {
    Absent,
    Blank,
    Json(Value),
}

enum RecordRead {
    Missing,
    Parsed(ContextRecord),
    Corrupt,
}

impl AgentContext {
    pub fn open(data_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                data_dir,
                state: Mutex::new(None),
            }),
        }
    }

    /// Remove context left behind when the app stopped before the shell exited.
    pub fn repair(&self, home: &Path) -> Result<(), String> {
        let mut guard = self.lock();
        self.clear_locked(&mut guard, home)
    }

    pub fn publish(&self, home: &Path, generation: u64, place: &AgentPlace) -> Result<(), String> {
        let mut guard = self.lock();
        if let Some(published) = guard.as_mut() {
            if published.generation == generation {
                published.place = place.clone();
                return write_text(
                    Path::new(&published.record.rule_path),
                    &rule_text(&published.place),
                    0o644,
                );
            }
        }
        self.clear_locked(&mut guard, home)?;
        let record = snapshot(home, place)?;
        write_record(&self.record_path(), &record)?;
        if let Err(err) = apply(&record, place) {
            if revert(&record).is_ok() {
                let _ = fs::remove_file(self.record_path());
            }
            return Err(err);
        }
        *guard = Some(Published {
            generation,
            place: place.clone(),
            record,
        });
        Ok(())
    }

    pub fn note_app_url(&self, url: &str) -> Result<(), String> {
        let mut guard = self.lock();
        let Some(published) = guard.as_mut() else {
            return Ok(());
        };
        if published.place.app_url == url {
            return Ok(());
        }
        published.place.app_url = url.to_string();
        write_text(
            Path::new(&published.record.rule_path),
            &rule_text(&published.place),
            0o644,
        )
    }

    pub fn clear_generation(&self, generation: u64) {
        let mut guard = self.lock();
        if guard
            .as_ref()
            .is_some_and(|published| published.generation != generation)
        {
            return;
        }
        let home = home_from(guard.as_ref());
        let _ = self.clear_locked(&mut guard, &home);
    }

    pub fn clear_all(&self) {
        let mut guard = self.lock();
        let home = home_from(guard.as_ref());
        let _ = self.clear_locked(&mut guard, &home);
    }

    fn clear_locked(&self, guard: &mut Option<Published>, home: &Path) -> Result<(), String> {
        match read_record(&self.record_path()) {
            RecordRead::Parsed(record) => {
                revert(&record)?;
                let _ = fs::remove_file(self.record_path());
                *guard = None;
                Ok(())
            }
            RecordRead::Corrupt => {
                let _ = fs::remove_file(self.record_path());
                best_effort(home)?;
                *guard = None;
                Ok(())
            }
            RecordRead::Missing => {
                if let Some(published) = guard.as_ref() {
                    revert(&published.record)?;
                }
                *guard = None;
                Ok(())
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Published>> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record_path(&self) -> PathBuf {
        self.inner.data_dir.join(RECORD_NAME)
    }
}

fn home_from(published: Option<&Published>) -> PathBuf {
    published
        .and_then(|published| Path::new(&published.record.mcp_path).parent())
        .and_then(|cursor| cursor.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn rule_text(place: &AgentPlace) -> String {
    let page = if place.app_url.is_empty() {
        "No page is open in the preview yet.".to_string()
    } else {
        format!("The preview was opened at {}.", place.app_url)
    };
    let api = if place.api_url.is_empty() {
        String::new()
    } else {
        format!("\nThe app API is {}.", place.api_url)
    };
    format!(
        "---\ndescription: Buho terminal\nalwaysApply: true\n---\n\n{RULE_MARKER}\n\n{page}{api}\n\nControl the preview with the MCP server named {SERVER_NAME}. Call list_pages, then navigate, then describe_page, then set_fields. Paths that contain :name need a real id from the open page. These tools change the window the user is looking at.\n\nCall describe_page before you change the page. Stay on that page unless the user asks you to open another one.\n\n{LEAVE_CONFIRMATION_TO_THE_USER}\n"
    )
}

fn snapshot(home: &Path, place: &AgentPlace) -> Result<ContextRecord, String> {
    let cursor = home.join(".cursor");
    let rules = cursor.join("rules");
    let mcp_path = cursor.join("mcp.json");
    let rule_path = rules.join(RULE_NAME);
    let original_mcp = read_optional(&mcp_path)?;
    let original_rule = read_optional(&rule_path)?;
    let (mcp_updated, replaced_server) = if place.mcp_url.is_empty() {
        (false, None)
    } else {
        let previous = match classify(&original_mcp)? {
            McpFile::Json(value) => value
                .get("mcpServers")
                .and_then(|servers| servers.get(SERVER_NAME))
                .cloned(),
            McpFile::Absent | McpFile::Blank => None,
        };
        (true, previous)
    };
    Ok(ContextRecord {
        mcp_path: mcp_path.display().to_string(),
        rule_path: rule_path.display().to_string(),
        original_mcp_b64: original_mcp.as_ref().map(|bytes| encode(bytes)),
        original_rule_b64: original_rule.as_ref().map(|bytes| encode(bytes)),
        replaced_server,
        mcp_updated,
        created_cursor: !cursor.exists(),
        created_rules: !rules.exists(),
    })
}

fn apply(record: &ContextRecord, place: &AgentPlace) -> Result<(), String> {
    if record.mcp_updated {
        let original = decode_optional(record.original_mcp_b64.as_deref())?;
        let mut value = match classify(&original)? {
            McpFile::Json(value) => value,
            McpFile::Absent | McpFile::Blank => json!({}),
        };
        let servers = value
            .as_object_mut()
            .ok_or_else(|| "Cursor's MCP config is not a JSON object.".to_string())?
            .entry("mcpServers")
            .or_insert_with(|| json!({}));
        let servers = servers
            .as_object_mut()
            .ok_or_else(|| "Cursor's MCP config has an unreadable mcpServers entry.".to_string())?;
        servers.insert(SERVER_NAME.to_string(), json!({ "url": place.mcp_url }));
        write_json(Path::new(&record.mcp_path), &value)?;
    }
    write_text(Path::new(&record.rule_path), &rule_text(place), 0o644)
}

fn revert(record: &ContextRecord) -> Result<(), String> {
    if record.mcp_updated {
        restore_mcp(record)?;
    }
    restore_rule(record)?;
    remove_empty_dirs(record);
    Ok(())
}

fn restore_mcp(record: &ContextRecord) -> Result<(), String> {
    let path = Path::new(&record.mcp_path);
    let current = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "Could not read Cursor's MCP config to remove the preview: {err}"
            ));
        }
    };
    let mut value = match classify(&Some(current)) {
        Ok(McpFile::Json(value)) => value,
        Ok(McpFile::Absent) | Ok(McpFile::Blank) => json!({}),
        Err(_) => return restore_original_bytes(path, record.original_mcp_b64.as_deref(), 0o600),
    };
    if let Some(servers) = value
        .get_mut("mcpServers")
        .and_then(|servers| servers.as_object_mut())
    {
        if let Some(previous) = &record.replaced_server {
            servers.insert(SERVER_NAME.to_string(), previous.clone());
        } else {
            servers.remove(SERVER_NAME);
        }
    }
    let original = decode_optional(record.original_mcp_b64.as_deref())?;
    let blank = match &original {
        None => true,
        Some(bytes) => String::from_utf8_lossy(bytes).trim().is_empty(),
    };
    if blank && mcp_servers_empty(&value) && only_server_map(&value) {
        return restore_original_bytes(path, record.original_mcp_b64.as_deref(), 0o600);
    }
    write_json(path, &value)
}

fn restore_rule(record: &ContextRecord) -> Result<(), String> {
    restore_original_bytes(
        Path::new(&record.rule_path),
        record.original_rule_b64.as_deref(),
        0o644,
    )
}

fn restore_original_bytes(path: &Path, encoded: Option<&str>, mode: u32) -> Result<(), String> {
    match encoded {
        None => {
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|err| format!("Could not remove {}: {err}", path.display()))?;
            }
            Ok(())
        }
        Some(encoded) => {
            let bytes = decode(encoded)?;
            write_bytes(path, &bytes, mode)
        }
    }
}

fn remove_empty_dirs(record: &ContextRecord) {
    let rule = Path::new(&record.rule_path);
    let mcp = Path::new(&record.mcp_path);
    if record.created_rules {
        if let Some(rules) = rule.parent() {
            remove_dir_if_empty(rules);
        }
    }
    if record.created_cursor {
        if let Some(cursor) = mcp.parent() {
            remove_dir_if_empty(cursor);
        }
    }
}

fn remove_dir_if_empty(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    if entries.count() == 0 {
        let _ = fs::remove_dir(path);
    }
}

fn best_effort(home: &Path) -> Result<(), String> {
    let rule = home.join(".cursor").join("rules").join(RULE_NAME);
    if let Ok(text) = fs::read_to_string(&rule) {
        if text.contains(RULE_MARKER) {
            let _ = fs::remove_file(&rule);
        }
    }
    let mcp = home.join(".cursor").join("mcp.json");
    let Ok(bytes) = fs::read(&mcp) else {
        return Ok(());
    };
    let Ok(McpFile::Json(mut value)) = classify(&Some(bytes)) else {
        return Ok(());
    };
    let Some(servers) = value
        .get_mut("mcpServers")
        .and_then(|servers| servers.as_object_mut())
    else {
        return Ok(());
    };
    if servers.remove(SERVER_NAME).is_some() {
        write_json(&mcp, &value)?;
    }
    Ok(())
}

fn classify(bytes: &Option<Vec<u8>>) -> Result<McpFile, String> {
    let Some(bytes) = bytes else {
        return Ok(McpFile::Absent);
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        return Ok(McpFile::Blank);
    }
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(value) if value.is_object() => {
            if let Some(servers) = value.get("mcpServers") {
                if !servers.is_null() && !servers.is_object() {
                    return Err(
                        "Cursor's MCP config has an unreadable mcpServers entry, so the preview tools were not added."
                            .into(),
                    );
                }
            }
            Ok(McpFile::Json(value))
        }
        Ok(_) => Err(
            "Cursor's MCP config is not a JSON object, so the preview tools were not added.".into(),
        ),
        Err(_) => Err(
            "Cursor's MCP config is not valid JSON, so the preview tools were not added.".into(),
        ),
    }
}

fn mcp_servers_empty(value: &Value) -> bool {
    match value.get("mcpServers") {
        None => true,
        Some(Value::Object(servers)) => servers.is_empty(),
        Some(_) => false,
    }
}

fn only_server_map(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| key == "mcpServers"))
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("Could not read {}: {err}", path.display())),
    }
}

fn read_record(path: &Path) -> RecordRead {
    match fs::read(path) {
        Err(_) => RecordRead::Missing,
        Ok(bytes) => match serde_json::from_slice::<ContextRecord>(&bytes) {
            Ok(record) => RecordRead::Parsed(record),
            Err(_) => RecordRead::Corrupt,
        },
    }
}

fn write_record(path: &Path, record: &ContextRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|err| format!("Could not store the terminal context: {err}"))?;
    write_bytes(path, &bytes, 0o600)
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
    text.push('\n');
    write_bytes(path, text.as_bytes(), 0o600)
}

fn write_text(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    write_bytes(path, text.as_bytes(), mode)
}

fn write_bytes(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Could not create {}: {err}", parent.display()))?;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("context");
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    {
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
        file.write_all(bytes)
            .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
        let _ = file.sync_all();
    }
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path).map_err(|err| format!("Could not write {}: {err}", path.display()))
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(encoded: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "The saved terminal context could not be read.".to_string())
}

fn decode_optional(encoded: Option<&str>) -> Result<Option<Vec<u8>>, String> {
    encoded.map(decode).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "app-shell-agent-context-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("home")).unwrap();
            fs::create_dir_all(path.join("data")).unwrap();
            Self(path)
        }

        fn home(&self) -> PathBuf {
            self.0.join("home")
        }

        fn data(&self) -> PathBuf {
            self.0.join("data")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn place() -> AgentPlace {
        AgentPlace {
            app_url: "https://app.example.com/home".into(),
            mcp_url: "http://127.0.0.1:47322/mcp?token=secret-token".into(),
            api_url: "http://127.0.0.1:3002".into(),
        }
    }

    #[test]
    fn the_rule_names_the_open_page_and_leaves_the_token_out() {
        let text = rule_text(&place());
        assert!(text.contains("alwaysApply: true"));
        assert!(text.contains("https://app.example.com/home"));
        assert!(text.contains(SERVER_NAME));
        assert!(text.contains("list_pages"));
        assert!(text.contains("describe_page"));
        assert!(text.contains(LEAVE_CONFIRMATION_TO_THE_USER));
        assert!(!text.contains("then submit_form"));
        assert!(text.contains("http://127.0.0.1:3002"));
        assert!(!text.contains("secret-token"));
        assert!(!text.contains("token="));
    }

    #[test]
    fn the_rule_says_when_no_page_is_open() {
        let text = rule_text(&AgentPlace {
            app_url: String::new(),
            mcp_url: String::new(),
            api_url: String::new(),
        });
        assert!(text.contains("No page is open in the preview yet."));
        assert!(!text.contains("The app API is"));
    }

    #[test]
    fn publishing_adds_the_preview_server_and_keeps_the_others() {
        let scratch = Scratch::new();
        let mcp = scratch.home().join(".cursor").join("mcp.json");
        fs::create_dir_all(mcp.parent().unwrap()).unwrap();
        fs::write(
            &mcp,
            "{\n  \"mcpServers\": {\n    \"notes\": { \"command\": \"notes\" }\n  }\n}\n",
        )
        .unwrap();
        let context = AgentContext::open(scratch.data());
        context.publish(&scratch.home(), 1, &place()).unwrap();

        let stored: Value = serde_json::from_str(&fs::read_to_string(&mcp).unwrap()).unwrap();
        assert_eq!(stored["mcpServers"]["notes"]["command"], "notes");
        assert_eq!(
            stored["mcpServers"][SERVER_NAME]["url"],
            "http://127.0.0.1:47322/mcp?token=secret-token"
        );
        let rule =
            fs::read_to_string(scratch.home().join(".cursor/rules").join(RULE_NAME)).unwrap();
        assert!(rule.contains("https://app.example.com/home"));
        assert!(!rule.contains("secret-token"));

        context
            .note_app_url("https://app.example.com/orders")
            .unwrap();
        let rule =
            fs::read_to_string(scratch.home().join(".cursor/rules").join(RULE_NAME)).unwrap();
        assert!(rule.contains("https://app.example.com/orders"));

        context.clear_generation(1);
        let stored: Value = serde_json::from_str(&fs::read_to_string(&mcp).unwrap()).unwrap();
        assert!(stored["mcpServers"].get(SERVER_NAME).is_none());
        assert_eq!(stored["mcpServers"]["notes"]["command"], "notes");
        assert!(
            !scratch
                .home()
                .join(".cursor/rules")
                .join(RULE_NAME)
                .exists()
        );
        assert!(!scratch.data().join(RECORD_NAME).exists());
    }

    #[test]
    fn clearing_restores_an_empty_mcp_file_and_a_previous_rule() {
        let scratch = Scratch::new();
        let cursor = scratch.home().join(".cursor");
        fs::create_dir_all(cursor.join("rules")).unwrap();
        fs::write(cursor.join("mcp.json"), "").unwrap();
        fs::write(cursor.join("rules").join(RULE_NAME), "user rule\n").unwrap();
        let context = AgentContext::open(scratch.data());
        context.publish(&scratch.home(), 4, &place()).unwrap();
        assert!(
            fs::read_to_string(cursor.join("rules").join(RULE_NAME))
                .unwrap()
                .contains(RULE_MARKER)
        );

        context.clear_generation(4);
        assert_eq!(fs::read_to_string(cursor.join("mcp.json")).unwrap(), "");
        assert_eq!(
            fs::read_to_string(cursor.join("rules").join(RULE_NAME)).unwrap(),
            "user rule\n"
        );
    }

    #[test]
    fn clearing_removes_a_config_the_terminal_created() {
        let scratch = Scratch::new();
        let context = AgentContext::open(scratch.data());
        context.publish(&scratch.home(), 2, &place()).unwrap();
        assert!(scratch.home().join(".cursor/mcp.json").exists());
        context.clear_generation(2);
        assert!(!scratch.home().join(".cursor").exists());
    }

    #[test]
    fn an_unreadable_mcp_config_is_left_unchanged() {
        let scratch = Scratch::new();
        let mcp = scratch.home().join(".cursor").join("mcp.json");
        fs::create_dir_all(mcp.parent().unwrap()).unwrap();
        fs::write(&mcp, "{ this is not json").unwrap();
        let context = AgentContext::open(scratch.data());
        let err = context.publish(&scratch.home(), 3, &place()).unwrap_err();
        assert!(err.contains("not valid JSON"));
        assert_eq!(fs::read_to_string(&mcp).unwrap(), "{ this is not json");
        assert!(
            !scratch
                .home()
                .join(".cursor/rules")
                .join(RULE_NAME)
                .exists()
        );
        assert!(!scratch.data().join(RECORD_NAME).exists());
    }

    #[test]
    fn a_previous_server_entry_is_restored() {
        let scratch = Scratch::new();
        let mcp = scratch.home().join(".cursor").join("mcp.json");
        fs::create_dir_all(mcp.parent().unwrap()).unwrap();
        fs::write(
            &mcp,
            format!(
                "{{\"mcpServers\":{{\"{SERVER_NAME}\":{{\"url\":\"http://old.example\"}},\"notes\":{{\"command\":\"notes\"}}}}}}\n"
            ),
        )
        .unwrap();
        let context = AgentContext::open(scratch.data());
        context.publish(&scratch.home(), 5, &place()).unwrap();
        context.clear_generation(5);
        let stored: Value = serde_json::from_str(&fs::read_to_string(&mcp).unwrap()).unwrap();
        assert_eq!(
            stored["mcpServers"][SERVER_NAME]["url"],
            "http://old.example"
        );
        assert_eq!(stored["mcpServers"]["notes"]["command"], "notes");
    }

    #[test]
    fn repair_removes_context_left_after_a_crash() {
        let scratch = Scratch::new();
        let context = AgentContext::open(scratch.data());
        context.publish(&scratch.home(), 8, &place()).unwrap();
        drop(context);
        let restarted = AgentContext::open(scratch.data());
        restarted.repair(&scratch.home()).unwrap();
        assert!(!scratch.home().join(".cursor").exists());
        assert!(!scratch.data().join(RECORD_NAME).exists());
    }

    #[test]
    fn an_older_shell_cannot_clear_the_new_context() {
        let scratch = Scratch::new();
        let context = AgentContext::open(scratch.data());
        context.publish(&scratch.home(), 1, &place()).unwrap();
        context
            .publish(
                &scratch.home(),
                2,
                &AgentPlace {
                    app_url: "https://app.example.com/next".into(),
                    ..place()
                },
            )
            .unwrap();
        context.clear_generation(1);
        let rule =
            fs::read_to_string(scratch.home().join(".cursor/rules").join(RULE_NAME)).unwrap();
        assert!(rule.contains("https://app.example.com/next"));
        context.clear_generation(2);
    }
}
