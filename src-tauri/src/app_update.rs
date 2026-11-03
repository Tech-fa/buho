use std::cmp::Ordering;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

pub const RELEASE_URL: &str = "https://buho.tech-fa.ca/api/release";
pub const SITE_URL: &str = "https://buho.tech-fa.ca/";
pub const DOWNLOAD_MAC_URL: &str = "https://buho.tech-fa.ca/download/mac";
pub const DOWNLOAD_WINDOWS_URL: &str = "https://buho.tech-fa.ca/download/windows";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub download_url: String,
}

pub fn check(current: &str) -> Result<AppUpdate, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .redirects(0)
        .build();
    let response = agent.get(RELEASE_URL).call().map_err(|_| {
        "We could not check for a newer Buho. Check the internet connection, then try again."
            .to_string()
    })?;
    let body = response.into_string().map_err(|_| {
        "We could not check for a newer Buho. Check the internet connection, then try again."
            .to_string()
    })?;
    Ok(notice_for(current, std::env::consts::OS, &body))
}

pub fn download_url_for(os: &str) -> &'static str {
    match os {
        "macos" => DOWNLOAD_MAC_URL,
        "windows" => DOWNLOAD_WINDOWS_URL,
        _ => SITE_URL,
    }
}

pub fn notice_for(current: &str, os: &str, body: &str) -> AppUpdate {
    let quiet = AppUpdate {
        current_version: current.to_string(),
        latest_version: String::new(),
        update_available: false,
        download_url: String::new(),
    };
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return quiet;
    };
    let Some(latest) = value.get("version").and_then(Value::as_str) else {
        return quiet;
    };
    let latest = latest.trim();
    if !is_newer(latest, current) {
        return AppUpdate {
            current_version: current.to_string(),
            latest_version: latest.to_string(),
            update_available: false,
            download_url: String::new(),
        };
    }
    AppUpdate {
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        update_available: true,
        download_url: download_url_for(os).to_string(),
    }
}

fn is_newer(latest: &str, current: &str) -> bool {
    let Some(latest_parts) = version_parts(latest) else {
        return false;
    };
    let Some(current_parts) = version_parts(current) else {
        return false;
    };
    cmp_parts(&latest_parts, &current_parts) == Ordering::Greater
}

fn version_parts(value: &str) -> Option<Vec<u64>> {
    let value = value.trim().trim_start_matches('v');
    if value.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for part in value.split('.') {
        if part.is_empty() || !part.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse().ok()?);
    }
    Some(parts)
}

fn cmp_parts(left: &[u64], right: &[u64]) -> Ordering {
    let len = left.len().max(right.len());
    for index in 0..len {
        let ordering = left
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&right.get(index).copied().unwrap_or(0));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"version":"1.0.5","publishedAt":"2026-11-03T00:00:00Z","notes":"Buho 1.0.5","downloads":[]}"#;

    #[test]
    fn a_newer_release_points_at_the_download_for_this_computer() {
        let mac = notice_for("1.0.4", "macos", BODY);
        assert!(mac.update_available);
        assert_eq!(mac.current_version, "1.0.4");
        assert_eq!(mac.latest_version, "1.0.5");
        assert_eq!(mac.download_url, DOWNLOAD_MAC_URL);

        let windows = notice_for("1.0.4", "windows", BODY);
        assert_eq!(windows.download_url, DOWNLOAD_WINDOWS_URL);

        let other = notice_for("1.0.4", "linux", BODY);
        assert_eq!(other.download_url, SITE_URL);
    }

    #[test]
    fn the_same_or_newer_copy_stays_quiet() {
        let same = notice_for("1.0.5", "macos", BODY);
        assert!(!same.update_available);
        assert!(same.download_url.is_empty());
        assert!(!notice_for("1.0.6", "macos", BODY).update_available);
        assert!(notice_for("1.0.9", "macos", r#"{"version":"1.0.10"}"#).update_available);
        assert!(!notice_for("1.0.10", "macos", r#"{"version":"1.0.9"}"#).update_available);
        assert!(!notice_for("1.0.4", "macos", r#"{"version":"latest"}"#).update_available);
        assert!(!notice_for("1.0.4", "macos", "not json").update_available);
        assert!(notice_for("1.0.4", "macos", r#"{"version":"v1.0.5"}"#).update_available);
    }
}
