use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn install(data_dir: &Path) {
    let path = data_dir.join("shell.log");
    if let Ok(mut slot) = LOG_FILE.lock() {
        *slot = Some(path.clone());
    }
    eprintln!("desktop-shell log: {}", path.display());
}

pub fn write(context: &str, detail: &str) {
    let detail = redact(&tail(detail, 20_000));
    let line = format!("{} {context}: {detail}", utc_stamp(unix_seconds()));
    eprintln!("desktop-shell {line}");
    let Some(path) = LOG_FILE.lock().ok().and_then(|slot| slot.clone()) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    #[cfg(unix)]
    let opened = {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)
    };
    #[cfg(not(unix))]
    let opened = OpenOptions::new().create(true).append(true).open(&path);
    if let Ok(mut file) = opened {
        let _ = writeln!(file, "{line}");
    }
}

pub fn logged<T>(context: &str, result: Result<T, String>) -> Result<T, String> {
    result.map_err(|err| {
        write(context, &err);
        err
    })
}

pub fn logged_plain<T>(context: &str, result: Result<T, String>) -> Result<T, String> {
    result.map_err(|err| {
        write(context, &err);
        crate::deploy::plain_failure(&err)
    })
}

fn tail(detail: &str, max_chars: usize) -> String {
    let count = detail.chars().count();
    if count <= max_chars {
        return detail.to_string();
    }
    let skip = count - max_chars;
    detail.chars().skip(skip).collect()
}

fn redact(detail: &str) -> String {
    let mut text = detail.to_string();
    for key in [
        "RESEND_API_KEY",
        "DB_PASSWORD",
        "ADMIN_PASSWORD",
        "ENCRYPTION_KEY",
        "SECRET_KEY",
        "DIGITALOCEAN_TOKEN",
        "DOCKERHUB_TOKEN",
    ] {
        text = redact_assignment(&text, key);
    }
    text = redact_prefix(&text, "Bearer ");
    text = redact_prefix(&text, "dop_v1_");
    redact_prefix(&text, "re_")
}

fn redact_assignment(text: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let mut out = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(&needle) {
        out.push_str(&rest[..index]);
        out.push_str(&needle);
        out.push_str("[redacted]");
        let after = &rest[index + needle.len()..];
        let skip = after.find(char::is_whitespace).unwrap_or(after.len());
        rest = &after[skip..];
    }
    out.push_str(rest);
    out
}

fn redact_prefix(text: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(prefix) {
        out.push_str(&rest[..index]);
        out.push_str(prefix);
        out.push_str("[redacted]");
        let after = &rest[index + prefix.len()..];
        let skip = after
            .find(|character: char| {
                character.is_whitespace() || character == '"' || character == '\''
            })
            .unwrap_or(after.len());
        rest = &after[skip..];
    }
    out.push_str(rest);
    out
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn utc_stamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let hour = time / 3600;
    let minute = (time % 3600) / 60;
    let second = time % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * doy + 2) / 153;
    let day = doy - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_formats_as_utc() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00:00Z");
    }

    #[test]
    fn secrets_are_removed_from_the_log_line() {
        let cleaned =
            redact("failed RESEND_API_KEY=re_live_secret dop_v1_abc token Bearer dop_v1_xyz");
        assert!(!cleaned.contains("re_live_secret"));
        assert!(!cleaned.contains("dop_v1_abc"));
        assert!(!cleaned.contains("dop_v1_xyz"));
        assert!(cleaned.contains("[redacted]"));
    }

    #[test]
    fn an_error_is_appended_to_the_log_file() {
        let dir = std::env::temp_dir().join(format!("shell-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        install(&dir);
        write(
            "make the app",
            "DigitalOcean returned 403: You are not authorized",
        );
        let text = fs::read_to_string(dir.join("shell.log")).unwrap();
        assert!(text.contains("make the app: DigitalOcean returned 403"));
        let _ = fs::remove_dir_all(&dir);
    }
}
