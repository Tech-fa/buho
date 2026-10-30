use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const APP_NAME: &str = "Buho";
pub const MANIFEST_FILE: &str = "shell.manifest.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    None,
    Disk,
    Persisted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    String,
    Url,
    Number,
    Boolean,
    Text,
}

impl Default for FieldType {
    fn default() -> Self {
        Self::String
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldSpec {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type", default)]
    pub kind: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub placeholder: String,
    #[serde(rename = "default", default)]
    pub default_value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub config_file: String,
    pub spa: bool,
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedUi {
    pub manifest: Manifest,
    pub manifest_found: bool,
    pub stored: Map<String, Value>,
    pub source: ConfigSource,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub ui_path: String,
    pub ui_source: String,
    pub manifest_found: bool,
    pub app_name: String,
    pub app_id: String,
    pub config_file: String,
    pub fields: Vec<FieldSpec>,
    pub values: Map<String, Value>,
    pub missing_required: Vec<String>,
    pub ready_to_open: bool,
    pub status: String,
}

pub fn prepare_ui_dir(path: &Path) -> Result<PathBuf, String> {
    let canon = path.canonicalize().map_err(|_| {
        format!(
            "Could not open {}. Choose the folder that contains the built web app.",
            path.display()
        )
    })?;
    if !canon.is_dir() {
        return Err("Choose a folder.".into());
    }
    if !canon.join("index.html").is_file() {
        return Err(
            "That folder has no index.html. Choose the built web app folder, not the source folder."
                .into(),
        );
    }
    Ok(canon)
}

pub fn load_ui(data_dir: &Path, ui_dir: &Path) -> Result<LoadedUi, String> {
    let ui_dir = prepare_ui_dir(ui_dir)?;
    let manifest_path = ui_dir.join(MANIFEST_FILE);
    if manifest_path.is_file() {
        let manifest = read_manifest(&manifest_path, &ui_dir)?;
        let (stored, source) = read_initial_values(data_dir, &ui_dir, &manifest)?;
        return Ok(LoadedUi {
            manifest,
            manifest_found: true,
            stored,
            source,
        });
    }

    let id = sanitize_id(
        &ui_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".into()),
    );
    let name = ui_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Web app".into());
    let manifest = Manifest {
        id,
        name,
        config_file: "config.json".into(),
        spa: true,
        fields: Vec::new(),
    };
    let (stored, source) = read_initial_values(data_dir, &ui_dir, &manifest)?;
    let mut loaded = LoadedUi {
        manifest,
        manifest_found: false,
        stored,
        source,
    };
    refresh_inferred_fields(&mut loaded);
    Ok(loaded)
}

pub fn save_values(
    data_dir: &Path,
    loaded: &mut LoadedUi,
    incoming: &Value,
) -> Result<(), BTreeMap<String, String>> {
    let object = match incoming.as_object() {
        Some(object) => object,
        None => return Err(single("", "Settings must be a JSON object.".into())),
    };
    let normalized = normalize(loaded, object)?;
    let path = config_store_path(data_dir, &loaded.manifest.id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| single("", format!("Could not create the settings folder: {err}")))?;
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(normalized.clone()))
        .map_err(|err| single("", format!("Could not write settings: {err}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|err| single("", format!("Could not write settings: {err}")))?;
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    fs::rename(&tmp, &path)
        .map_err(|err| single("", format!("Could not write settings: {err}")))?;
    loaded.stored = normalized;
    loaded.source = ConfigSource::Persisted;
    refresh_inferred_fields(loaded);
    Ok(())
}

pub fn remember_ui_path(data_dir: &Path, ui_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .map_err(|err| format!("Could not create the app data folder: {err}"))?;
    fs::write(
        data_dir.join("ui-path.txt"),
        ui_dir.to_string_lossy().as_bytes(),
    )
    .map_err(|err| format!("Could not remember the web app folder: {err}"))
}

pub fn remembered_ui_path(data_dir: &Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(data_dir.join("ui-path.txt")).ok()?;
    let path = PathBuf::from(raw.trim());
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

impl LoadedUi {
    pub fn is_ready(&self) -> bool {
        let served = match self.source {
            ConfigSource::None => Map::new(),
            ConfigSource::Disk | ConfigSource::Persisted => self.stored.clone(),
        };
        validate_values(&self.manifest, &served, self.manifest_found).is_empty()
    }
}

pub fn empty_view(status: &str) -> AppView {
    AppView {
        ui_path: String::new(),
        ui_source: "none".into(),
        manifest_found: false,
        app_name: APP_NAME.into(),
        app_id: String::new(),
        config_file: "config.json".into(),
        fields: Vec::new(),
        values: Map::new(),
        missing_required: Vec::new(),
        ready_to_open: false,
        status: status.into(),
    }
}

pub fn view(ui_dir: &Path, ui_source: &str, loaded: &LoadedUi, status: &str) -> AppView {
    let served = match loaded.source {
        ConfigSource::None => Map::new(),
        ConfigSource::Disk | ConfigSource::Persisted => loaded.stored.clone(),
    };
    let errors = validate_values(&loaded.manifest, &served, loaded.manifest_found);
    AppView {
        ui_path: ui_dir.display().to_string(),
        ui_source: ui_source.into(),
        manifest_found: loaded.manifest_found,
        app_name: loaded.manifest.name.clone(),
        app_id: loaded.manifest.id.clone(),
        config_file: loaded.manifest.config_file.clone(),
        fields: display_fields(loaded),
        values: form_values(loaded),
        missing_required: errors.keys().cloned().collect(),
        ready_to_open: loaded.is_ready(),
        status: status.into(),
    }
}

fn refresh_inferred_fields(loaded: &mut LoadedUi) {
    if loaded.manifest_found {
        return;
    }
    loaded.manifest.fields = infer_fields(&loaded.stored);
}

fn display_fields(loaded: &LoadedUi) -> Vec<FieldSpec> {
    let mut fields = loaded.manifest.fields.clone();
    let mut extras: Vec<String> = loaded
        .stored
        .keys()
        .filter(|key| !fields.iter().any(|field| &field.key == *key))
        .cloned()
        .collect();
    extras.sort();
    for key in extras {
        if let Some(value) = loaded.stored.get(&key) {
            fields.push(infer_one(&key, value));
        }
    }
    fields
}

fn form_values(loaded: &LoadedUi) -> Map<String, Value> {
    let mut values = Map::new();
    for field in display_fields(loaded) {
        if let Some(value) = loaded.stored.get(&field.key) {
            values.insert(field.key, value.clone());
        } else if let Some(default_value) = field.default_value {
            values.insert(field.key, default_value);
        } else if field.kind == FieldType::Boolean {
            values.insert(field.key, Value::Bool(false));
        }
    }
    values
}

fn read_manifest(path: &Path, ui_dir: &Path) -> Result<Manifest, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("Could not read {MANIFEST_FILE}: {err}"))?;
    let parsed: ManifestFile = serde_json::from_str(&text)
        .map_err(|err| format!("{MANIFEST_FILE} is not valid: {err}"))?;
    let folder_name = ui_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".into());
    let id = sanitize_id(if parsed.id.trim().is_empty() {
        &folder_name
    } else {
        parsed.id.trim()
    });
    let name = if parsed.name.trim().is_empty() {
        folder_name
    } else {
        parsed.name.trim().to_string()
    };
    let config_file = normalize_config_file(&parsed.config_file)?;
    let mut seen = BTreeMap::<String, ()>::new();
    for field in &parsed.fields {
        if !valid_setting_key(&field.key) {
            return Err(format!(
                "{MANIFEST_FILE} has a setting name that is not a plain identifier: {}.",
                field.key
            ));
        }
        if seen.insert(field.key.clone(), ()).is_some() {
            return Err(format!(
                "{MANIFEST_FILE} repeats the setting {}.",
                field.key
            ));
        }
        if let Some(default_value) = &field.default_value {
            if check_field(field, Some(default_value)).is_err() {
                return Err(format!(
                    "{MANIFEST_FILE} default for {} does not match its type.",
                    field.key
                ));
            }
        }
    }
    Ok(Manifest {
        id,
        name,
        config_file,
        spa: parsed.spa,
        fields: parsed.fields,
    })
}

fn read_initial_values(
    data_dir: &Path,
    ui_dir: &Path,
    manifest: &Manifest,
) -> Result<(Map<String, Value>, ConfigSource), String> {
    let persisted_path = config_store_path(data_dir, &manifest.id);
    if persisted_path.is_file() {
        return Ok((
            read_object(&persisted_path).map_err(|err| {
                format!("Saved settings for {} are unreadable: {err}", manifest.id)
            })?,
            ConfigSource::Persisted,
        ));
    }
    let disk_path = ui_dir.join(&manifest.config_file);
    if disk_path.is_file() {
        return match read_object(&disk_path) {
            Ok(values) => Ok((values, ConfigSource::Disk)),
            Err(err) => Err(format!(
                "{} in the web app is not a JSON object: {err}",
                manifest.config_file
            )),
        };
    }
    Ok((Map::new(), ConfigSource::None))
}

fn read_object(path: &Path) -> Result<Map<String, Value>, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    match serde_json::from_str::<Value>(&text).map_err(|err| err.to_string())? {
        Value::Object(values) => Ok(values),
        _ => Err("expected a JSON object".into()),
    }
}

fn normalize(
    loaded: &LoadedUi,
    incoming: &Map<String, Value>,
) -> Result<Map<String, Value>, BTreeMap<String, String>> {
    let mut errors = BTreeMap::new();
    let mut values = Map::new();
    if loaded.manifest_found {
        for field in &loaded.manifest.fields {
            match check_field(field, incoming.get(&field.key)) {
                Ok(Some(value)) => {
                    values.insert(field.key.clone(), value);
                }
                Ok(None) => {}
                Err(message) => {
                    errors.insert(field.key.clone(), message);
                }
            }
        }
        for (key, value) in incoming {
            if loaded.manifest.fields.iter().any(|field| &field.key == key) {
                continue;
            }
            match scalar_extra(key, value) {
                Ok(Some(value)) => {
                    values.insert(key.clone(), value);
                }
                Ok(None) => {}
                Err(message) => {
                    errors.insert(key.clone(), message);
                }
            }
        }
    } else {
        for (key, value) in incoming {
            match scalar_extra(key, value) {
                Ok(Some(value)) => {
                    values.insert(key.clone(), value);
                }
                Ok(None) => {}
                Err(message) => {
                    errors.insert(key.clone(), message);
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}

fn validate_values(
    manifest: &Manifest,
    values: &Map<String, Value>,
    manifest_found: bool,
) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    if manifest_found {
        for field in &manifest.fields {
            if let Err(message) = check_field(field, values.get(&field.key)) {
                errors.insert(field.key.clone(), message);
            }
        }
    }
    errors
}

fn check_field(field: &FieldSpec, value: Option<&Value>) -> Result<Option<Value>, String> {
    let Some(value) = value else {
        return if field.required {
            Err(format!("{} is required.", field.label))
        } else {
            Ok(None)
        };
    };
    if matches!(value, Value::Null) {
        return if field.required {
            Err(format!("{} is required.", field.label))
        } else {
            Ok(None)
        };
    }
    match field.kind {
        FieldType::String | FieldType::Text => match value.as_str() {
            Some(text) => {
                let text = text.trim();
                if text.is_empty() {
                    if field.required {
                        Err(format!("{} is required.", field.label))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(Some(Value::String(text.to_string())))
                }
            }
            None => Err(format!("{} must be text.", field.label)),
        },
        FieldType::Url => match value.as_str() {
            Some(text) => {
                let text = text.trim();
                if text.is_empty() {
                    if field.required {
                        Err(format!("{} is required.", field.label))
                    } else {
                        Ok(None)
                    }
                } else if valid_http_url(text) {
                    Ok(Some(Value::String(text.to_string())))
                } else {
                    Err(format!(
                        "{} must start with http:// or https://.",
                        field.label
                    ))
                }
            }
            None => Err(format!("{} must be a URL.", field.label)),
        },
        FieldType::Number => match finite_number(value) {
            Ok(number) => Ok(Some(number)),
            Err(_) => Err(format!("{} must be a number.", field.label)),
        },
        FieldType::Boolean => match value.as_bool() {
            Some(flag) => Ok(Some(Value::Bool(flag))),
            None => Err(format!("{} must be on or off.", field.label)),
        },
    }
}

fn scalar_extra(key: &str, value: &Value) -> Result<Option<Value>, String> {
    if !valid_setting_key(key) {
        return Err(format!("{key} is not a plain setting name."));
    }
    match value {
        Value::Null => Ok(None),
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(Value::String(text.to_string())))
            }
        }
        Value::Number(_) => finite_number(value)
            .map(Some)
            .map_err(|_| format!("{key} must be a finite number.")),
        Value::Bool(flag) => Ok(Some(Value::Bool(*flag))),
        _ => Err(format!("{key} must be text, a number, or on/off.")),
    }
}

fn infer_fields(values: &Map<String, Value>) -> Vec<FieldSpec> {
    let mut keys: Vec<&String> = values.keys().collect();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| values.get(key).map(|value| infer_one(key, value)))
        .collect()
}

fn infer_one(key: &str, value: &Value) -> FieldSpec {
    let kind = match value {
        Value::Bool(_) => FieldType::Boolean,
        Value::Number(_) => FieldType::Number,
        Value::String(text) if text.contains('\n') => FieldType::Text,
        _ => FieldType::String,
    };
    FieldSpec {
        key: key.into(),
        label: key.into(),
        description: String::new(),
        kind,
        required: false,
        placeholder: String::new(),
        default_value: None,
    }
}

fn normalize_config_file(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_start_matches(['/', '\\']);
    if trimmed.is_empty() {
        return Ok("config.json".into());
    }
    let path = Path::new(trimmed);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{MANIFEST_FILE} configFile must stay inside the web app folder."
        ));
    }
    Ok(trimmed.replace('\\', "/"))
}

pub fn sanitize_id(raw: &str) -> String {
    let mut out = String::new();
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            out.push(character.to_ascii_lowercase());
        } else if character.is_whitespace() || character == '.' {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "app".into() } else { out }
}

fn valid_setting_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

fn finite_number(value: &Value) -> Result<Value, ()> {
    let number = value.as_f64().ok_or(())?;
    if !number.is_finite() {
        return Err(());
    }
    if let Value::Number(_) = value {
        return Ok(value.clone());
    }
    Ok(Value::Number(
        serde_json::Number::from_f64(number).ok_or(())?,
    ))
}

fn valid_http_url(value: &str) -> bool {
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"));
    let Some(rest) = rest else {
        return false;
    };
    if rest.is_empty() || rest.contains(' ') {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    !host.is_empty() && !host.starts_with('.')
}

fn config_store_path(data_dir: &Path, app_id: &str) -> PathBuf {
    data_dir
        .join("apps")
        .join(sanitize_id(app_id))
        .join("config.json")
}

fn single(key: &str, message: String) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    errors.insert(key.into(), message);
    errors
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default = "default_config_file")]
    config_file: String,
    #[serde(default = "default_true")]
    spa: bool,
    #[serde(default)]
    fields: Vec<FieldSpec>,
}

fn default_config_file() -> String {
    "config.json".into()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "app-shell-{}-{}-{}",
                std::process::id(),
                nanos,
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn manifest_declares_the_settings_and_save_persists_them() {
        let scratch = Scratch::new();
        let ui = scratch.path().join("ui");
        let data = scratch.path().join("data");
        write(&ui.join("index.html"), "<!doctype html><title>App</title>");
        write(
            &ui.join(MANIFEST_FILE),
            r#"{
              "id": "4es-dashboard",
              "name": "4ES",
              "configFile": "config.json",
              "fields": [
                {
                  "key": "apiUrl",
                  "label": "API URL",
                  "type": "url",
                  "required": true
                },
                {
                  "key": "featureContextOfOrganization",
                  "label": "Context of the organization",
                  "type": "boolean",
                  "default": false
                }
              ]
            }"#,
        );

        let mut loaded = load_ui(&data, &ui).unwrap();
        assert!(loaded.manifest_found);
        assert_eq!(loaded.manifest.id, "4es-dashboard");
        assert_eq!(loaded.source, ConfigSource::None);
        let seen = view(&ui, "saved", &loaded, "");
        assert!(!seen.ready_to_open);
        assert_eq!(
            seen.values.get("featureContextOfOrganization"),
            Some(&Value::Bool(false))
        );

        let mut incoming = Map::new();
        incoming.insert(
            "apiUrl".into(),
            Value::String("http://localhost:3002".into()),
        );
        incoming.insert("featureContextOfOrganization".into(), Value::Bool(true));
        save_values(&data, &mut loaded, &Value::Object(incoming)).unwrap();
        assert_eq!(loaded.source, ConfigSource::Persisted);
        assert!(view(&ui, "saved", &loaded, "").ready_to_open);

        let again = load_ui(&data, &ui).unwrap();
        assert_eq!(again.source, ConfigSource::Persisted);
        assert_eq!(
            again.stored.get("apiUrl").and_then(Value::as_str),
            Some("http://localhost:3002")
        );
    }

    #[test]
    fn rejects_a_bad_url_and_a_manifest_that_escapes_the_folder() {
        let scratch = Scratch::new();
        let ui = scratch.path().join("ui");
        let data = scratch.path().join("data");
        write(&ui.join("index.html"), "<!doctype html>");
        write(
            &ui.join(MANIFEST_FILE),
            r#"{
              "id": "Demo",
              "name": "Demo",
              "fields": [
                {"key": "apiUrl", "label": "API URL", "type": "url", "required": true}
              ]
            }"#,
        );
        let mut loaded = load_ui(&data, &ui).unwrap();
        assert_eq!(loaded.manifest.id, "demo");
        let mut incoming = Map::new();
        incoming.insert("apiUrl".into(), Value::String("localhost:3002".into()));
        let errors = save_values(&data, &mut loaded, &Value::Object(incoming)).unwrap_err();
        assert!(errors.get("apiUrl").unwrap().contains("http://"));

        write(
            &ui.join(MANIFEST_FILE),
            r#"{"id":"demo","name":"Demo","configFile":"../secret.json","fields":[]}"#,
        );
        let err = load_ui(&data, &ui).unwrap_err();
        assert!(err.contains("configFile"));
    }

    #[test]
    fn infers_settings_when_the_app_has_no_manifest() {
        let scratch = Scratch::new();
        let ui = scratch.path().join("custom-ui");
        let data = scratch.path().join("data");
        write(&ui.join("index.html"), "<!doctype html>");
        write(
            &ui.join("config.json"),
            r#"{"siteTitle":"North","retries":2,"enabled":true}"#,
        );
        let loaded = load_ui(&data, &ui).unwrap();
        assert!(!loaded.manifest_found);
        assert_eq!(loaded.source, ConfigSource::Disk);
        assert_eq!(loaded.manifest.id, "custom-ui");
        assert!(view(&ui, "saved", &loaded, "").ready_to_open);
        let kinds: Vec<_> = loaded
            .manifest
            .fields
            .iter()
            .map(|field| (field.key.as_str(), field.kind.clone()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("enabled", FieldType::Boolean),
                ("retries", FieldType::Number),
                ("siteTitle", FieldType::String),
            ]
        );
    }
}
