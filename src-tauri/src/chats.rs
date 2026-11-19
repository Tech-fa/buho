use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatRecord {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub open: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatSummary {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub open: bool,
    pub updated_at: u64,
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("chats")
}

pub fn load_all(folder: &Path) -> Result<Vec<ChatRecord>, String> {
    if !folder.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    let entries = fs::read_dir(folder).map_err(|err| err.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).map_err(|err| err.to_string())?;
        let record: ChatRecord = match serde_json::from_slice(&bytes) {
            Ok(record) => record,
            Err(_) => continue,
        };
        records.push(record);
    }
    records.sort_by_key(|record| record.updated_at);
    Ok(records)
}

pub fn list(folder: &Path) -> Result<Vec<ChatSummary>, String> {
    let mut records = load_all(folder)?;
    records.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
    Ok(records.into_iter().map(summary_of).collect())
}

pub fn create(folder: &Path, id: &str, session_id: &str) -> Result<ChatRecord, String> {
    let path = file_for(folder, session_id)?;
    if path.exists() {
        return read_record(&path);
    }
    let now = now_ms();
    let record = ChatRecord {
        id: id.to_string(),
        session_id: session_id.to_string(),
        title: "New chat".into(),
        provider: "auto".into(),
        model: String::new(),
        open: true,
        created_at: now,
        updated_at: now,
        messages: Vec::new(),
    };
    write_record(&path, &record)?;
    Ok(record)
}

pub fn set_open(folder: &Path, session_id: &str, open: bool) -> Result<(), String> {
    let path = file_for(folder, session_id)?;
    if !path.exists() {
        return Ok(());
    }
    let mut record = read_record(&path)?;
    record.open = open;
    record.updated_at = now_ms();
    write_record(&path, &record)
}

pub fn append_user(
    folder: &Path,
    id: &str,
    session_id: &str,
    text: &str,
    provider: &str,
) -> Result<ChatRecord, String> {
    let path = file_for(folder, session_id)?;
    let mut record = if path.exists() {
        read_record(&path)?
    } else {
        create(folder, id, session_id)?
    };
    record.messages.push(json!({ "role": "user", "content": text }));
    record.provider = provider.to_string();
    record.open = true;
    if record.title == "New chat" {
        if let Some(title) = title_from_text(text) {
            record.title = title;
        }
    }
    record.updated_at = now_ms();
    write_record(&path, &record)?;
    Ok(record)
}

pub fn save_context(
    folder: &Path,
    session_id: &str,
    messages: &[Value],
    provider: &str,
    model: &str,
) -> Result<ChatRecord, String> {
    let path = file_for(folder, session_id)?;
    let mut record = if path.exists() {
        read_record(&path)?
    } else {
        create(folder, session_id, session_id)?
    };
    record.messages = messages.to_vec();
    if !provider.is_empty() {
        record.provider = provider.to_string();
    }
    if !model.is_empty() {
        record.model = model.to_string();
    }
    if record.title == "New chat" {
        if let Some(title) = title_from_messages(&record.messages) {
            record.title = title;
        }
    }
    record.updated_at = now_ms();
    write_record(&path, &record)?;
    Ok(record)
}

pub fn load(folder: &Path, session_id: &str) -> Result<ChatRecord, String> {
    read_record(&file_for(folder, session_id)?)
}

#[tauri::command]
pub fn agent_history(shell: State<'_, crate::Shell>) -> Result<Vec<ChatSummary>, String> {
    list(&dir(shell.data_dir()))
}

#[tauri::command]
pub fn agent_transcript(
    shell: State<'_, crate::Shell>,
    session_id: String,
) -> Result<ChatRecord, String> {
    load(&dir(shell.data_dir()), &session_id)
}

fn summary_of(record: ChatRecord) -> ChatSummary {
    ChatSummary {
        id: record.id,
        session_id: record.session_id,
        title: record.title,
        provider: record.provider,
        model: record.model,
        open: record.open,
        updated_at: record.updated_at,
    }
}

fn file_for(folder: &Path, session_id: &str) -> Result<PathBuf, String> {
    if !session_ok(session_id) {
        return Err("That chat id is not valid.".into());
    }
    Ok(folder.join(format!("{session_id}.json")))
}

fn session_ok(session_id: &str) -> bool {
    let mut chars = session_id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    session_id.len() <= 80
        && session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn read_record(path: &Path) -> Result<ChatRecord, String> {
    let bytes = fs::read(path).map_err(|_| "That chat is not in the history.".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "That chat could not be read.".to_string())
}

fn write_record(path: &Path, record: &ChatRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let body = serde_json::to_vec_pretty(record).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body).map_err(|err| err.to_string())?;
    fs::rename(&tmp, path).map_err(|err| err.to_string())?;
    Ok(())
}

fn title_from_messages(messages: &[Value]) -> Option<String> {
    messages.iter().find_map(|message| {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            return None;
        }
        message
            .get("content")
            .and_then(Value::as_str)
            .and_then(title_from_text)
    })
}

fn title_from_text(text: &str) -> Option<String> {
    let line = text.lines().next()?.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    let mut chars = line.chars();
    let short = chars.by_ref().take(42).collect::<String>();
    if chars.next().is_some() {
        Some(format!("{short}…"))
    } else {
        Some(short)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "buho-chats-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn a_closed_chat_stays_in_the_history() {
        let folder = scratch();
        create(&folder, "agent-1", "session-1").unwrap();
        let saved = append_user(&folder, "agent-1", "session-1", "Where is the login page?", "anthropic").unwrap();
        assert_eq!(saved.title, "Where is the login page?");
        assert_eq!(saved.messages.len(), 1);
        set_open(&folder, "session-1", false).unwrap();
        let listed = list(&folder).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].open);
        assert_eq!(listed[0].title, "Where is the login page?");
        let loaded = load(&folder, "session-1").unwrap();
        assert_eq!(loaded.messages.len(), 1);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_saved_context_keeps_the_whole_thread() {
        let folder = scratch();
        append_user(&folder, "agent-2", "session-2", "Hello", "auto").unwrap();
        let messages = vec![
            json!({ "role": "user", "content": "Hello" }),
            json!({
                "role": "assistant",
                "content": [{ "type": "text", "text": "Hi" }, { "type": "tool_use", "id": "t1", "name": "tabs.list", "input": {} }]
            }),
            json!({
                "role": "user",
                "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "page-1" }]
            }),
        ];
        let saved = save_context(&folder, "session-2", &messages, "openai", "gpt-4o").unwrap();
        assert_eq!(saved.messages.len(), 3);
        assert_eq!(saved.provider, "openai");
        assert_eq!(saved.model, "gpt-4o");
        assert_eq!(load(&folder, "session-2").unwrap().messages, messages);
        let _ = fs::remove_dir_all(&folder);
    }
}
