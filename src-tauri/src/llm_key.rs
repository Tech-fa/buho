use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const FILE_NAME: &str = "buho-llm-key";

static INSTALLED: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn install(dir: &Path) {
    *INSTALLED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(dir.to_path_buf());
}

pub fn key() -> String {
    let installed = INSTALLED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let env_value = std::env::var("BUHO_LLM_KEY").ok();
    key_in(installed.as_deref(), env_value.as_deref())
}

pub fn is_set() -> bool {
    !key().is_empty()
}

pub fn save(dir: &Path, raw: &str) -> Result<(), String> {
    let token = raw.trim();
    validate(token)?;
    write_secret(&key_path(dir), token)
}

pub fn clear(dir: &Path) -> Result<(), String> {
    write_secret(&key_path(dir), "")
}

fn key_in(dir: Option<&Path>, env_value: Option<&str>) -> String {
    if let Some(dir) = dir {
        if let Some(saved) = saved(dir) {
            return saved;
        }
    }
    env_value.unwrap_or("").trim().to_string()
}

fn saved(dir: &Path) -> Option<String> {
    match fs::read_to_string(key_path(dir)) {
        Ok(text) => Some(text.trim().to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    }
}

fn validate(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("Paste the buho_llm token.".into());
    }
    if token.len() > 512
        || token
            .chars()
            .any(|char| char.is_whitespace() || char.is_control())
    {
        return Err("That buho_llm token is not valid.".into());
    }
    Ok(())
}

fn key_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

fn write_secret(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "Could not save the buho_llm token.".to_string())?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&tmp)
            .map_err(|_| "Could not save the buho_llm token.".to_string())?;
        file.write_all(contents.as_bytes())
            .map_err(|_| "Could not save the buho_llm token.".to_string())?;
    }
    fs::rename(&tmp, path).map_err(|_| "Could not save the buho_llm token.".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "buho-llm-key-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn a_saved_token_is_used_before_the_environment() {
        let dir = scratch();
        save(&dir, " sk-saved ").unwrap();
        assert_eq!(key_in(Some(&dir), Some("sk-env")), "sk-saved");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_environment_is_used_until_someone_saves_a_token() {
        let dir = scratch();
        assert_eq!(key_in(Some(&dir), Some("sk-env")), "sk-env");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_the_token_turns_it_off() {
        let dir = scratch();
        save(&dir, "sk-saved").unwrap();
        clear(&dir).unwrap();
        assert_eq!(key_in(Some(&dir), Some("sk-env")), "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blank_token_is_refused() {
        let dir = scratch();
        assert!(save(&dir, "  ").is_err());
        assert!(save(&dir, "sk-not a token").is_err());
        assert!(key_in(Some(&dir), None).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_token_is_private_to_this_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        save(&dir, "sk-saved").unwrap();
        let mode = fs::metadata(key_path(&dir)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }
}
