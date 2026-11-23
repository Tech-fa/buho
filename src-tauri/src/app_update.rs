use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const RELEASE_URL: &str = "https://buho.tech-fa.ca/api/release";
pub const SITE_URL: &str = "https://buho.tech-fa.ca/";
pub const DOWNLOAD_MAC_URL: &str = "https://buho.tech-fa.ca/download/mac";
pub const DOWNLOAD_WINDOWS_URL: &str = "https://buho.tech-fa.ca/download/windows";

const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Download {
    platform: String,
    file: String,
    url: String,
    sha256: String,
}

pub fn check(current: &str) -> Result<AppUpdate, String> {
    let body = fetch_body()?;
    Ok(notice_for(current, std::env::consts::OS, &body))
}

pub fn install(current: &str) -> Result<(), String> {
    let body = fetch_body()?;
    let download = installer_for(current, std::env::consts::OS, std::env::consts::ARCH, &body)?;
    #[cfg(target_os = "macos")]
    let bundle = installed_bundle()?;
    let dir = std::env::temp_dir().join(format!("buho-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|_| "Could not prepare the update.".to_string())?;
    let result = (|| {
        let file = dir.join(&download.file);
        let sum = download_file(&download.url, &file)?;
        if !sum.eq_ignore_ascii_case(&download.sha256) {
            return Err("The download did not match the published update.".into());
        }
        #[cfg(target_os = "macos")]
        {
            swap_mac(&file, &bundle)
        }
        #[cfg(target_os = "windows")]
        {
            swap_windows(&file)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = file;
            Err("This computer cannot install the update.".into())
        }
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
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
    let Some(download) = pick_download(os, std::env::consts::ARCH, &downloads_from(&value)) else {
        return AppUpdate {
            current_version: current.to_string(),
            latest_version: latest.to_string(),
            update_available: false,
            download_url: String::new(),
        };
    };
    AppUpdate {
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        update_available: true,
        download_url: download.url,
    }
}

fn installer_for(current: &str, os: &str, arch: &str, body: &str) -> Result<Download, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| "We could not read the update. Try again in a few minutes.".to_string())?;
    let latest = value
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !is_newer(latest, current) {
        return Err("This Buho is already the newest.".into());
    }
    pick_download(os, arch, &downloads_from(&value))
        .ok_or_else(|| "There is no update for this computer.".into())
}

fn fetch_body() -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .redirects(0)
        .build();
    let response = agent.get(RELEASE_URL).call().map_err(|_| {
        "We could not check for a newer Buho. Check the internet connection, then try again."
            .to_string()
    })?;
    response.into_string().map_err(|_| {
        "We could not check for a newer Buho. Check the internet connection, then try again."
            .to_string()
    })
}

fn downloads_from(value: &Value) -> Vec<Download> {
    value
        .get("downloads")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(download_from).collect())
        .unwrap_or_default()
}

fn download_from(value: &Value) -> Option<Download> {
    let platform = value.get("platform")?.as_str()?.trim().to_string();
    let file = value.get("file")?.as_str()?.trim().to_string();
    let url = value.get("url")?.as_str()?.trim().to_string();
    let sha256 = value.get("sha256")?.as_str()?.trim().to_string();
    if platform.is_empty() || safe_file_name(&file).is_none() || !valid_sha256(&sha256) {
        return None;
    }
    https_url(&url).ok()?;
    Some(Download {
        platform,
        file,
        url,
        sha256,
    })
}

fn pick_download(os: &str, arch: &str, downloads: &[Download]) -> Option<Download> {
    let mut items: Vec<&Download> = downloads
        .iter()
        .filter(|item| item.platform == os)
        .collect();
    if os == "windows" {
        let programs: Vec<&Download> = items
            .iter()
            .copied()
            .filter(|item| item.file.to_ascii_lowercase().ends_with(".exe"))
            .collect();
        if !programs.is_empty() {
            items = programs;
        }
    }
    if os == "macos" {
        items.retain(|item| item.file.to_ascii_lowercase().ends_with(".dmg"));
    }
    if let Some(item) = items
        .iter()
        .copied()
        .find(|item| arch_matches(&item.file, arch))
    {
        return Some(item.clone());
    }
    if items.len() == 1 && !names_an_arch(&items[0].file) {
        return Some(items[0].clone());
    }
    None
}

fn arch_matches(file: &str, arch: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    let tokens: &[&str] = match arch {
        "aarch64" | "arm64" => &["aarch64", "arm64"],
        "x86_64" => &["x64", "x86_64", "amd64"],
        _ => return false,
    };
    tokens.iter().any(|token| lower.contains(token))
}

fn names_an_arch(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    ["aarch64", "arm64", "x64", "x86_64", "amd64"]
        .iter()
        .any(|token| lower.contains(token))
}

fn safe_file_name(value: &str) -> Option<&str> {
    let name = Path::new(value).file_name()?.to_str()?;
    if name != value || name.is_empty() || name.starts_with('.') {
        return None;
    }
    Some(name)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn https_url(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|_| "The update address is not valid.".to_string())?;
    if url.scheme() != "https" || !url.username().is_empty() || url.host_str().is_none() {
        return Err("The update address is not valid.".into());
    }
    Ok(())
}

fn download_file(url: &str, dest: &Path) -> Result<String, String> {
    https_url(url)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(600))
        .redirects(0)
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|_| "Could not download the update.".to_string())?;
    if response.status() != 200 {
        return Err("Could not download the update.".into());
    }
    if response
        .header("content-length")
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_BYTES)
    {
        return Err("The update is larger than expected.".into());
    }
    let mut reader = response.into_reader();
    let mut file = File::create(dest).map_err(|_| "Could not save the update.".to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|_| "Could not download the update.".to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_BYTES {
            return Err("The update is larger than expected.".into());
        }
        hasher.update(&buf[..read]);
        file.write_all(&buf[..read])
            .map_err(|_| "Could not save the update.".to_string())?;
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
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

#[cfg(target_os = "macos")]
fn installed_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "This copy cannot be updated.".to_string())?;
    let bundle = exe
        .parent()
        .and_then(|dir| dir.parent())
        .and_then(|dir| dir.parent())
        .map(Path::to_path_buf)
        .ok_or_else(|| "This copy cannot be updated.".to_string())?;
    if bundle.extension().and_then(|ext| ext.to_str()) != Some("app") {
        return Err("Open the installed Buho app to update it.".into());
    }
    let parent = bundle
        .parent()
        .ok_or_else(|| "This copy cannot be updated.".to_string())?;
    let probe = parent.join(".buho-update-probe");
    fs::write(&probe, b"ok").map_err(|_| {
        "Buho cannot replace this copy. Move it into your Applications folder, then try again."
            .to_string()
    })?;
    let _ = fs::remove_file(&probe);
    Ok(bundle)
}

#[cfg(target_os = "macos")]
fn swap_mac(dmg: &Path, bundle: &Path) -> Result<(), String> {
    let work = dmg
        .parent()
        .ok_or_else(|| "Could not prepare the update.".to_string())?;
    let mount = work.join("mount");
    fs::create_dir_all(&mount).map_err(|_| "Could not prepare the update.".to_string())?;
    let mut attach = Command::new("hdiutil");
    attach
        .arg("attach")
        .arg("-nobrowse")
        .arg("-readonly")
        .arg("-mountpoint")
        .arg(&mount)
        .arg(dmg);
    command_ok(&mut attach, "Could not open the update.")?;
    let copied: Result<PathBuf, String> = (|| {
        let source = find_bundle(&mount)?;
        let staged = work.join(
            source
                .file_name()
                .ok_or_else(|| "The update did not contain the app.".to_string())?,
        );
        let mut copy = Command::new("ditto");
        copy.arg(&source).arg(&staged);
        command_ok(&mut copy, "Could not copy the update.")?;
        let _ = Command::new("xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(&staged)
            .status();
        Ok(staged)
    })();
    let _ = Command::new("hdiutil")
        .args(["detach", "-quiet"])
        .arg(&mount)
        .status();
    let staged = copied?;
    let script = work.join("swap.sh");
    fs::write(&script, MAC_SWAP).map_err(|_| "Could not prepare the update.".to_string())?;
    let mut command = Command::new("/bin/bash");
    command
        .arg(&script)
        .arg(std::process::id().to_string())
        .arg(bundle)
        .arg(&staged);
    spawn_detached(&mut command)
}

#[cfg(target_os = "macos")]
const MAC_SWAP: &str = r#"#!/bin/bash
set -euo pipefail
pid="$1"
target="$2"
staged="$3"
while kill -0 "$pid" 2>/dev/null; do
  sleep 0.2
done
sleep 0.5
next="${target}.next"
old="${target}.old"
rm -rf "$next"
ditto "$staged" "$next"
rm -rf "$old"
if ! mv "$target" "$old"; then
  rm -rf "$next"
  exit 1
fi
if ! mv "$next" "$target"; then
  mv "$old" "$target"
  exit 1
fi
rm -rf "$old"
open "$target"
rm -rf "$(dirname "$staged")"
"#;

#[cfg(target_os = "macos")]
fn find_bundle(mount: &Path) -> Result<PathBuf, String> {
    let mut found = None;
    for entry in fs::read_dir(mount).map_err(|_| "Could not read the update.".to_string())? {
        let path = entry
            .map_err(|_| "Could not read the update.".to_string())?
            .path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("app") && path.is_dir() {
            if found.is_some() {
                return Err("The update contained more than one app.".into());
            }
            found = Some(path);
        }
    }
    found.ok_or_else(|| "The update did not contain the app.".into())
}

#[cfg(target_os = "windows")]
fn swap_windows(installer: &Path) -> Result<(), String> {
    let work = installer
        .parent()
        .ok_or_else(|| "Could not prepare the update.".to_string())?;
    let script = work.join("swap.cmd");
    fs::write(&script, WINDOWS_SWAP).map_err(|_| "Could not prepare the update.".to_string())?;
    let mut command = Command::new("cmd");
    command
        .arg("/C")
        .arg(&script)
        .arg(std::process::id().to_string())
        .arg(installer);
    spawn_detached(&mut command)
}

#[cfg(target_os = "windows")]
const WINDOWS_SWAP: &str = "@echo off\r\nset PID=%~1\r\nset INSTALLER=%~2\r\n:wait\r\ntasklist /FI \"PID eq %PID%\" | find \"%PID%\" >nul\r\nif not errorlevel 1 (\r\n  ping -n 2 127.0.0.1 >nul\r\n  goto wait\r\n)\r\n\"%INSTALLER%\" /S\r\n";

fn spawn_detached(command: &mut Command) -> Result<(), String> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000008 | 0x08000000);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not start the update.".into())
}

fn command_ok(command: &mut Command, failure: &str) -> Result<(), String> {
    let output = command.output().map_err(|_| failure.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        Err(failure.to_string())
    } else {
        let detail: String = detail.chars().take(300).collect();
        Err(format!("{failure} {detail}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"version":"1.0.5","publishedAt":"2026-11-03T00:00:00Z","notes":"Buho 1.0.5","downloads":[]}"#;

    fn release(version: &str, downloads: &str) -> String {
        format!(r#"{{"version":"{version}","downloads":[{downloads}]}}"#)
    }

    fn file(platform: &str, name: &str, url: &str) -> String {
        format!(
            r#"{{"platform":"{platform}","file":"{name}","url":"{url}","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#
        )
    }

    #[test]
    fn a_newer_release_points_at_the_installer_for_this_computer() {
        let body = release(
            "1.0.5",
            &format!(
                "{},{},{}",
                file("macos", "Buho.dmg", "https://example.com/Buho.dmg"),
                file(
                    "windows",
                    "Buho-setup.exe",
                    "https://example.com/Buho-setup.exe"
                ),
                file("windows", "Buho.msi", "https://example.com/Buho.msi")
            ),
        );
        let notice = notice_for("1.0.4", std::env::consts::OS, &body);
        assert_eq!(notice.current_version, "1.0.4");
        assert_eq!(notice.latest_version, "1.0.5");
        if cfg!(target_os = "macos") {
            assert!(notice.update_available);
            assert_eq!(notice.download_url, "https://example.com/Buho.dmg");
        } else if cfg!(target_os = "windows") {
            assert!(notice.update_available);
            assert_eq!(notice.download_url, "https://example.com/Buho-setup.exe");
        } else {
            assert!(!notice.update_available);
        }
    }

    #[test]
    fn the_installer_matches_this_processors_kind() {
        let downloads = downloads_from(
            &serde_json::from_str::<Value>(&release(
                "1.0.5",
                &format!(
                    "{},{}",
                    file("macos", "Buho_aarch64.dmg", "https://example.com/arm.dmg"),
                    file("macos", "Buho_x64.dmg", "https://example.com/intel.dmg")
                ),
            ))
            .unwrap(),
        );
        assert_eq!(
            pick_download("macos", "aarch64", &downloads).unwrap().url,
            "https://example.com/arm.dmg"
        );
        assert_eq!(
            pick_download("macos", "x86_64", &downloads).unwrap().url,
            "https://example.com/intel.dmg"
        );
    }

    #[test]
    fn the_same_or_newer_copy_stays_quiet() {
        let same = notice_for("1.0.5", "macos", BODY);
        assert!(!same.update_available);
        assert!(same.download_url.is_empty());
        assert!(!notice_for("1.0.6", "macos", BODY).update_available);
        let newer = release(
            "1.0.10",
            &file("macos", "Buho.dmg", "https://example.com/Buho.dmg"),
        );
        assert!(notice_for("1.0.9", "macos", &newer).update_available);
        assert!(!notice_for("1.0.10", "macos", &newer).update_available);
        assert!(!notice_for("1.0.4", "macos", r#"{"version":"latest"}"#).update_available);
        assert!(!notice_for("1.0.4", "macos", "not json").update_available);
        let prefixed = release(
            "v1.0.5",
            &file("macos", "Buho.dmg", "https://example.com/Buho.dmg"),
        );
        assert!(notice_for("1.0.4", "macos", &prefixed).update_available);
        assert!(!notice_for("1.0.4", "macos", BODY).update_available);
    }

    #[test]
    fn a_bad_address_or_checksum_is_ignored() {
        let body = release(
            "2.0.0",
            &format!(
                "{},{}",
                r#"{"platform":"macos","file":"../Buho.dmg","url":"https://example.com/Buho.dmg","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
                r#"{"platform":"macos","file":"Buho.dmg","url":"http://example.com/Buho.dmg","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
            ),
        );
        assert!(!notice_for("1.0.0", "macos", &body).update_available);
    }

    #[test]
    fn a_saved_file_has_a_stable_checksum() {
        let dir = std::env::temp_dir().join(format!("buho-sum-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hello.txt");
        fs::write(&file, b"hello").unwrap();
        let sum = download_checksum(&file);
        assert_eq!(
            sum,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn download_checksum(path: &Path) -> String {
        let bytes = fs::read(path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex(&hasher.finalize())
    }
}
