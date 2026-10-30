use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const FILE_NAME: &str = "deployment.json";
const API: &str = "https://api.digitalocean.com/v2";
pub const BILLING_URL: &str = "https://cloud.digitalocean.com/account/billing";
pub const TOKEN_URL: &str = "https://cloud.digitalocean.com/account/api/tokens";
pub const RESEND_URL: &str = "https://resend.com/api-keys";
pub const DROPLET_SIZE: &str = "s-2vcpu-4gb";
const DEFAULT_FROM_EMAIL: &str = "notifications@resend.dev";
const API_IMAGE: &str = "techfa22/fourss-api";
const UI_IMAGE: &str = "techfa22/fourss-ui";
const COMPOSE_FILE: &str = include_str!("../../deploy/docker-compose.yml");
const DEPLOY_SCRIPT: &str = include_str!("../../deploy/deploy.sh");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfig {
    #[serde(default)]
    pub digital_ocean_token: String,
    #[serde(default)]
    pub resend_api_key: String,
    #[serde(default)]
    pub from_email: String,
    #[serde(default)]
    pub admin_email: String,
    #[serde(default)]
    pub admin_password: String,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub ssh_public_key: String,
    #[serde(default)]
    pub ssh_private_key: String,
    #[serde(default)]
    pub ssh_key_id: String,
    #[serde(default)]
    pub droplet_id: u64,
    #[serde(default)]
    pub droplet_name: String,
    #[serde(default)]
    pub public_ip: String,
    #[serde(default)]
    pub db_password: String,
    #[serde(default)]
    pub encryption_key: String,
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub ui_version: String,
}

fn default_region() -> String {
    "nyc3".into()
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            digital_ocean_token: String::new(),
            resend_api_key: String::new(),
            from_email: String::new(),
            admin_email: String::new(),
            admin_password: String::new(),
            region: default_region(),
            size: String::new(),
            ssh_public_key: String::new(),
            ssh_private_key: String::new(),
            ssh_key_id: String::new(),
            droplet_id: 0,
            droplet_name: String::new(),
            public_ip: String::new(),
            db_password: String::new(),
            encryption_key: String::new(),
            secret_key: String::new(),
            api_version: String::new(),
            ui_version: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentInput {
    pub digital_ocean_token: String,
    pub resend_api_key: String,
    pub from_email: String,
    pub admin_email: String,
    pub region: String,
    pub size: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentView {
    pub has_digital_ocean_token: bool,
    pub has_resend_api_key: bool,
    pub region: String,
    pub size: String,
    pub droplet_id: u64,
    pub droplet_name: String,
    pub public_ip: String,
    pub from_email: String,
    pub admin_email: String,
    pub admin_password: String,
    pub status: String,
    pub running: bool,
    pub app_url: String,
    pub api_version: String,
    pub ui_version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseVersions {
    pub api_version: String,
    pub ui_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionChoice {
    pub slug: String,
    pub name: String,
}

pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub fn ensure_config(data_dir: &Path) -> Result<DeploymentConfig, String> {
    fs::create_dir_all(data_dir)
        .map_err(|err| format!("Could not create the app data folder: {err}"))?;
    let path = config_path(data_dir);
    if !path.is_file() {
        let config = DeploymentConfig::default();
        write_config(&path, &config)?;
        return Ok(config);
    }
    read_config(&path)
}

pub fn save_input(data_dir: &Path, input: DeploymentInput) -> Result<DeploymentConfig, String> {
    let mut config = ensure_config(data_dir)?;
    if let Some(token) = nonempty(&input.digital_ocean_token) {
        reject_secret_text("DigitalOcean API token", token)?;
        config.digital_ocean_token = token.to_string();
    }
    if let Some(key) = nonempty(&input.resend_api_key) {
        reject_secret_text("Resend API key", key)?;
        config.resend_api_key = key.to_string();
    }
    if let Some(email) = nonempty(&input.from_email) {
        let email = effective_from_email(email);
        if !acceptable_email(&email) {
            return Err("Type the address people should see when the app emails them.".into());
        }
        config.from_email = email;
    }
    if let Some(email) = nonempty(&input.admin_email) {
        let email = email.trim().to_lowercase();
        if !acceptable_email(&email) {
            return Err(
                "Type an email address you can open, such as the one you use for work.".into(),
            );
        }
        config.admin_email = email;
    }
    if let Some(region) = nonempty(&input.region) {
        if !valid_region(region) {
            return Err("Choose a place close to your team.".into());
        }
        config.region = region.to_string();
    }
    config.size = DROPLET_SIZE.into();
    write_config(&config_path(data_dir), &config)?;
    Ok(config)
}

pub fn delete_droplet(data_dir: &Path) -> Result<DeploymentConfig, String> {
    let mut config = ensure_config(data_dir)?;
    if config.droplet_id != 0 {
        let path = format!("/droplets/{}", config.droplet_id);
        match do_request(&config.digital_ocean_token, "DELETE", &path, None) {
            Ok(_) => {}
            Err(err) if err.contains("404") => {}
            Err(err) => return Err(err),
        }
    }
    config.droplet_id = 0;
    config.public_ip.clear();
    write_config(&config_path(data_dir), &config)?;
    Ok(config)
}

pub fn forget_resource(data_dir: &Path, kind: &str) -> Result<DeploymentConfig, String> {
    let mut config = ensure_config(data_dir)?;
    match kind {
        "token" => {
            if config.droplet_id != 0 {
                return Err(
                    "Remove the computer first. After that, you can remove the DigitalOcean key."
                        .into(),
                );
            }
            config.digital_ocean_token.clear();
        }
        "resend" => config.resend_api_key.clear(),
        _ => return Err("That item cannot be removed.".into()),
    }
    write_config(&config_path(data_dir), &config)?;
    Ok(config)
}

pub fn verify_token(token: &str) -> Result<(), String> {
    if token.trim().is_empty() {
        return Err("Paste the key from DigitalOcean.".into());
    }
    do_request(token.trim(), "GET", "/account", None).map(|_| ())
}

pub fn export_configuration(data_dir: &Path, destination: &Path) -> Result<(), String> {
    if destination.as_os_str().is_empty() {
        return Err("Choose where to save the copy.".into());
    }
    if destination == config_path(data_dir) {
        return Err("Choose a different folder for the copy.".into());
    }
    let config = ensure_config(data_dir)?;
    if config.public_ip.is_empty() {
        return Err("Finish making your app before you save a copy.".into());
    }
    let bytes = serde_json::to_vec_pretty(&config).map_err(|err| err.to_string())?;
    write_private_file(destination, &bytes)
}

pub fn list_regions(token: &str) -> Result<Vec<RegionChoice>, String> {
    let body = do_request(token, "GET", "/regions?per_page=200", None)?;
    let regions = body
        .get("regions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "DigitalOcean did not offer any places. Try again in a moment.".to_string()
        })?;
    let mut listed = Vec::new();
    for region in regions {
        if !region_is_offered(region) {
            continue;
        }
        let Some(slug) = region.get("slug").and_then(Value::as_str) else {
            continue;
        };
        let name = region.get("name").and_then(Value::as_str).unwrap_or(slug);
        listed.push(RegionChoice {
            slug: slug.to_string(),
            name: name.to_string(),
        });
    }
    listed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(listed)
}

pub fn view(config: DeploymentConfig, status: &str, running: bool) -> DeploymentView {
    let app_url = if config.public_ip.is_empty() {
        String::new()
    } else {
        format!("http://{}", config.public_ip)
    };
    DeploymentView {
        has_digital_ocean_token: !config.digital_ocean_token.is_empty(),
        has_resend_api_key: !config.resend_api_key.is_empty(),
        region: config.region,
        size: if config.size.is_empty() {
            DROPLET_SIZE.into()
        } else {
            config.size
        },
        droplet_id: config.droplet_id,
        droplet_name: config.droplet_name,
        public_ip: config.public_ip,
        from_email: effective_from_email(&config.from_email),
        admin_email: shown_email(&config.admin_email),
        admin_password: config.admin_password,
        status: status.to_string(),
        running,
        app_url,
        api_version: config.api_version,
        ui_version: config.ui_version,
    }
}

pub fn available_versions() -> Result<ReleaseVersions, String> {
    Ok(ReleaseVersions {
        api_version: highest_published(API_IMAGE)?,
        ui_version: highest_published(UI_IMAGE)?,
    })
}

pub fn provision(
    data_dir: &Path,
    report: &mut dyn FnMut(String),
) -> Result<DeploymentConfig, String> {
    let mut config = ensure_config(data_dir)?;
    require_field(&config.digital_ocean_token, "The DigitalOcean key")?;
    require_field(&config.resend_api_key, "The email key")?;
    require_field(&config.region, "A place close to your team")?;
    if !acceptable_email(&config.admin_email) {
        return Err("Type the email address you will use to sign in.".into());
    }
    config.from_email = effective_from_email(&config.from_email);
    if !acceptable_email(&config.from_email) {
        return Err("Type the address people should see when the app emails them.".into());
    }
    let release = available_versions()?;
    config.api_version = release.api_version;
    config.ui_version = release.ui_version;
    write_config(&config_path(data_dir), &config)?;
    config.size = DROPLET_SIZE.into();
    let new_app = config.droplet_id == 0;
    config.encryption_key = encryption_key_for_app(&config.encryption_key, new_app);
    if config.admin_password.is_empty() {
        config.admin_password = random_token(20);
    }
    if config.db_password.is_empty() {
        config.db_password = random_token(24);
    }
    if config.secret_key.is_empty() {
        config.secret_key = random_token(32);
    }
    if config.droplet_name.is_empty() {
        config.droplet_name = "documents-backend".into();
    }
    write_config(&config_path(data_dir), &config)?;

    if config.ssh_private_key.is_empty() || config.ssh_public_key.is_empty() {
        report(working_message());
        generate_ssh_key(data_dir, &mut config)?;
        write_config(&config_path(data_dir), &config)?;
    }
    if config.ssh_key_id.is_empty() {
        report(working_message());
        let created = do_request(
            &config.digital_ocean_token,
            "POST",
            "/account/keys",
            Some(json!({
                "name": "documents-shell",
                "public_key": config.ssh_public_key,
            })),
        )?;
        config.ssh_key_id = created
            .pointer("/ssh_key/id")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            .ok_or_else(|| "DigitalOcean did not finish saving the key. Try again.".to_string())?;
        write_config(&config_path(data_dir), &config)?;
    }

    if config.droplet_id == 0 || config.public_ip.is_empty() {
        report(working_message());
        let key_id: u64 = config
            .ssh_key_id
            .parse()
            .map_err(|_| "The saved SSH key id is not a number.".to_string())?;
        let created = do_request(
            &config.digital_ocean_token,
            "POST",
            "/droplets",
            Some(json!({
                "name": config.droplet_name,
                "region": config.region,
                "size": config.size,
                "image": "ubuntu-24-04-x64",
                "ssh_keys": [key_id],
                "tags": ["documents-backend"],
            })),
        )?;
        config.droplet_id = created
            .pointer("/droplet/id")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                "DigitalOcean did not finish creating the computer. Try again.".to_string()
            })?;
        write_config(&config_path(data_dir), &config)?;
        report(working_message());
        config.public_ip = wait_for_ip(&config)?;
        require_ip(&config.public_ip)?;
        write_config(&config_path(data_dir), &config)?;
    }
    require_ip(&config.public_ip)?;

    let key_path = write_private_key(data_dir, &config.ssh_private_key)?;
    let known_hosts = data_dir.join("known_hosts");
    report(working_message());
    wait_for_ssh(&config.public_ip)?;
    let _ = ssh_run(
        &key_path,
        &known_hosts,
        &config.public_ip,
        "cloud-init status --wait || true",
    );

    report(working_message());
    upload_runtime(
        data_dir,
        &key_path,
        &known_hosts,
        &config.public_ip,
        &config.api_version,
        &config.ui_version,
    )?;
    upload_env(&config, data_dir, &key_path, &known_hosts)?;

    report(working_message());
    ssh_run(
        &key_path,
        &known_hosts,
        &config.public_ip,
        &format!(
            "PUBLIC_IP={} bash /opt/fourss/desktop-shell/deploy/deploy.sh",
            shell_quote(&config.public_ip)
        ),
    )?;
    report("Your app is ready.".into());
    Ok(config)
}

fn require_field(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} is needed before we can make your app."))
    } else {
        Ok(())
    }
}

fn wait_for_ip(config: &DeploymentConfig) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    let path = format!("/droplets/{}", config.droplet_id);
    loop {
        let body = do_request(&config.digital_ocean_token, "GET", &path, None)?;
        if let Some(ip) = public_ip(&body) {
            return Ok(ip);
        }
        if Instant::now() > deadline {
            return Err(
                "The computer is taking too long to come online. Wait a moment, then try again."
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn public_ip(body: &Value) -> Option<String> {
    let networks = body.pointer("/droplet/networks/v4")?.as_array()?;
    networks.iter().find_map(|network| {
        let kind = network.get("type").and_then(Value::as_str)?;
        if kind == "public" {
            network
                .get("ip_address")
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            None
        }
    })
}

fn wait_for_ssh(ip: &str) -> Result<(), String> {
    let address: SocketAddr = format!("{ip}:22")
        .parse()
        .map_err(|_| format!("{ip} is not a usable IP address."))?;
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, Duration::from_secs(3)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    Err("SSH did not open on the droplet in time.".into())
}

fn generate_ssh_key(data_dir: &Path, config: &mut DeploymentConfig) -> Result<(), String> {
    let dir = data_dir.join("ssh");
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let key = dir.join("id_ed25519");
    let _ = fs::remove_file(&key);
    let _ = fs::remove_file(dir.join("id_ed25519.pub"));
    let status = Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-f",
            &key.to_string_lossy(),
            "-N",
            "",
            "-C",
            "documents-shell",
        ])
        .status()
        .map_err(|err| format!("ssh-keygen failed: {err}"))?;
    if !status.success() {
        return Err("ssh-keygen could not create a key.".into());
    }
    config.ssh_private_key = fs::read_to_string(&key).map_err(|err| err.to_string())?;
    config.ssh_public_key =
        fs::read_to_string(dir.join("id_ed25519.pub")).map_err(|err| err.to_string())?;
    config.ssh_public_key = config.ssh_public_key.trim().to_string();
    Ok(())
}

fn write_private_key(data_dir: &Path, private_key: &str) -> Result<PathBuf, String> {
    let path = data_dir.join("ssh").join("id_ed25519");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|err| err.to_string())?;
        file.write_all(private_key.as_bytes())
            .map_err(|err| err.to_string())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&path, private_key).map_err(|err| err.to_string())?;
    }
    Ok(path)
}

fn pinned_compose(api_version: &str, ui_version: &str) -> String {
    COMPOSE_FILE
        .replace(
            &format!("{API_IMAGE}:latest"),
            &format!("{API_IMAGE}:{api_version}"),
        )
        .replace(
            &format!("{UI_IMAGE}:latest"),
            &format!("{UI_IMAGE}:{ui_version}"),
        )
}

fn highest_published(image: &str) -> Result<String, String> {
    let tags = docker_hub_tags(image)?;
    highest_version(&tags).ok_or_else(|| {
        "The newest app is not published yet. Try again after the next build.".into()
    })
}

fn docker_hub_tags(image: &str) -> Result<Vec<String>, String> {
    let mut url = format!("https://hub.docker.com/v2/repositories/{image}/tags?page_size=100");
    let mut tags = Vec::new();
    for _ in 0..10 {
        if !url.starts_with("https://hub.docker.com/") {
            break;
        }
        let response = match ureq::get(&url).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(404, _)) => {
                return Err(
                    "The newest app is not published yet. Try again after the next build.".into(),
                );
            }
            Err(_) => {
                return Err(
                    "We could not find the newest app. Check the internet connection, then try again."
                        .into(),
                );
            }
        };
        let text = response.into_string().unwrap_or_default();
        let body: Value = serde_json::from_str(&text).map_err(|_| {
            "We could not find the newest app. Check the internet connection, then try again."
                .to_string()
        })?;
        if let Some(results) = body.get("results").and_then(Value::as_array) {
            for result in results {
                if let Some(name) = result.get("name").and_then(Value::as_str) {
                    tags.push(name.to_string());
                }
            }
        }
        match body.get("next").and_then(Value::as_str) {
            Some(next) if !next.is_empty() => url = next.to_string(),
            _ => break,
        }
    }
    Ok(tags)
}

fn highest_version(tags: &[String]) -> Option<String> {
    let mut best: Option<(Vec<u64>, String)> = None;
    for tag in tags {
        let Some(parts) = parse_version(tag) else {
            continue;
        };
        let replace = match &best {
            None => true,
            Some((current, _)) => cmp_version(&parts, current) == Ordering::Greater,
        };
        if replace {
            best = Some((parts, tag.clone()));
        }
    }
    best.map(|(_, tag)| tag)
}

fn parse_version(tag: &str) -> Option<Vec<u64>> {
    if tag.is_empty() || tag == "latest" {
        return None;
    }
    let mut parts = Vec::new();
    for part in tag.split('.') {
        if part.is_empty() || !part.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse().ok()?);
    }
    if parts.is_empty() { None } else { Some(parts) }
}

fn cmp_version(left: &[u64], right: &[u64]) -> Ordering {
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

fn upload_runtime(
    data_dir: &Path,
    key: &Path,
    known_hosts: &Path,
    ip: &str,
    api_version: &str,
    ui_version: &str,
) -> Result<(), String> {
    ssh_run(
        key,
        known_hosts,
        ip,
        "mkdir -p /opt/fourss/desktop-shell/deploy",
    )?;
    let compose_path = data_dir.join("docker-compose.yml");
    let script_path = data_dir.join("deploy.sh");
    let compose = pinned_compose(api_version, ui_version);
    fs::write(&compose_path, compose).map_err(|err| err.to_string())?;
    fs::write(&script_path, DEPLOY_SCRIPT).map_err(|err| err.to_string())?;
    let destination = format!("{}:/opt/fourss/desktop-shell/deploy/", ssh_destination(ip)?);
    let status = scp_command(key, known_hosts)
        .arg(&compose_path)
        .arg(&script_path)
        .arg(&destination)
        .status()
        .map_err(|err| format!("scp failed to start: {err}"))?;
    if !status.success() {
        return Err("Could not upload the deployment files.".into());
    }
    ssh_run(
        key,
        known_hosts,
        ip,
        "chmod 755 /opt/fourss/desktop-shell/deploy/deploy.sh",
    )
}

fn upload_env(
    config: &DeploymentConfig,
    data_dir: &Path,
    key: &Path,
    known_hosts: &Path,
) -> Result<(), String> {
    let env_path = data_dir.join("droplet.env");
    let contents = format!(
        "DB_HOST=mysql\nDB_PORT=3306\nDB_USER=documents\nDB_PASSWORD={db}\nDB_NAME=documents\nNODE_ENV=production\nPORT=3012\nRESEND_API_KEY={resend}\nFRONTEND_URL=http://{ip}\nFILE_STORAGE_DIR=storage\nFROM_EMAIL={from}\nENCRYPTION_KEY={encryption}\nENVIRONMENT=production\nADMIN_EMAIL={admin}\nADMIN_PASSWORD={admin_password}\nSECRET_KEY={secret}\nREDIS_URI=redis://redis:6379\nRABBIT_HOST=amqp://guest:guest@rabbitmq:5672\nPUBLIC_IP={ip}\n",
        db = config.db_password,
        resend = config.resend_api_key,
        ip = config.public_ip,
        from = config.from_email,
        encryption = config.encryption_key,
        admin = config.admin_email,
        admin_password = config.admin_password,
        secret = config.secret_key,
    );
    write_private_file(&env_path, contents.as_bytes())?;
    let destination = format!(
        "{}:/opt/fourss/desktop-shell/deploy/.env",
        ssh_destination(&config.public_ip)?
    );
    let status = scp_command(key, known_hosts)
        .arg(&env_path)
        .arg(&destination)
        .status()
        .map_err(|err| format!("scp failed to start: {err}"))?;
    if !status.success() {
        return Err("Could not upload the environment file.".into());
    }
    ssh_run(
        key,
        known_hosts,
        &config.public_ip,
        "chmod 600 /opt/fourss/desktop-shell/deploy/.env",
    )
}

fn ssh_run(key: &Path, known_hosts: &Path, ip: &str, remote: &str) -> Result<(), String> {
    ssh_destination(ip)?;
    let output = ssh_command(key, known_hosts, ip)
        .arg(remote)
        .output()
        .map_err(|err| format!("ssh failed to start: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Remote command failed: {}",
            command_output(&output)
        ))
    }
}

fn command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut text = String::new();
    if !stdout.trim().is_empty() {
        text.push_str(stdout.trim());
    }
    if !stderr.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(stderr.trim());
    }
    if text.is_empty() {
        format!("exit {}", output.status)
    } else {
        text
    }
}

fn ssh_command(key: &Path, known_hosts: &Path, ip: &str) -> Command {
    let mut command = Command::new("ssh");
    command.args(ssh_base_args(key, known_hosts));
    command.arg(ssh_destination(ip).unwrap_or_else(|_| "root@invalid".into()));
    command
}

fn scp_command(key: &Path, known_hosts: &Path) -> Command {
    let mut command = Command::new("scp");
    command.args(ssh_base_args(key, known_hosts));
    command
}

fn ssh_base_args(key: &Path, known_hosts: &Path) -> Vec<String> {
    vec![
        "-i".into(),
        key.display().to_string(),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", known_hosts.display()),
    ]
}

fn do_request(token: &str, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let url = format!("{API}{path}");
    let request = match method {
        "GET" => ureq::get(&url),
        "POST" => ureq::post(&url),
        "DELETE" => ureq::delete(&url),
        _ => return Err(format!("Unsupported method {method}.")),
    };
    let response = if let Some(body) = body {
        request
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_json(body)
    } else {
        request
            .set("Authorization", &format!("Bearer {token}"))
            .call()
    };
    match response {
        Ok(response) => {
            let text = response.into_string().unwrap_or_default();
            if text.trim().is_empty() {
                Ok(Value::Null)
            } else {
                serde_json::from_str(&text)
                    .map_err(|err| format!("DigitalOcean returned unreadable JSON: {err}"))
            }
        }
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or(text);
            if message.to_lowercase().contains("payment") {
                Err(format!(
                    "{message} Add a card at {BILLING_URL} and try again."
                ))
            } else {
                Err(format!("DigitalOcean returned {code}: {message}"))
            }
        }
        Err(err) => Err(format!("DigitalOcean request failed: {err}")),
    }
}

fn write_config(path: &Path, config: &DeploymentConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(config).map_err(|err| err.to_string())?;
    write_private_file(path, &bytes)
}

fn read_config(path: &Path) -> Result<DeploymentConfig, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(&text).map_err(|err| format!("Deployment settings are unreadable: {err}"))
}

fn random_token(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut bytes = vec![0u8; len];
    let mut file = File::open("/dev/urandom").expect("urandom");
    file.read_exact(&mut bytes).expect("urandom");
    bytes
        .into_iter()
        .map(|byte| ALPHABET[(byte as usize) % ALPHABET.len()] as char)
        .collect()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn nonempty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn reject_secret_text(label: &str, value: &str) -> Result<(), String> {
    if value
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        Err(format!("{label} cannot contain spaces or line breaks."))
    } else if value.len() > 500 {
        Err(format!("{label} is too long."))
    } else {
        Ok(())
    }
}

fn working_message() -> String {
    "Working on making your app. This can take a while. Leave this window open.".into()
}

fn region_is_offered(region: &Value) -> bool {
    if region.get("available").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    region
        .get("sizes")
        .and_then(Value::as_array)
        .map(|sizes| sizes.iter().any(|size| size.as_str() == Some(DROPLET_SIZE)))
        .unwrap_or(false)
}

pub fn plain_failure(message: &str) -> String {
    let lower = message.to_lowercase();
    let place_unavailable = lower.contains("invalid region")
        || ((lower.contains("region") || lower.contains("size"))
            && (lower.contains("not available") || lower.contains("unavailable")));
    if place_unavailable {
        return "That place is not available. Choose another place close to your team, then try again.".into();
    }
    if lower.contains("not authorized") || lower.contains("returned 403") {
        return "DigitalOcean did not accept this key. Create a new key and choose Full Access, as in the picture.".into();
    }
    if lower.contains("payment") || lower.contains("billing") {
        return format!(
            "DigitalOcean needs a payment card on the account before it can create the computer. Add a card at {BILLING_URL}, then try again."
        );
    }
    if lower.contains("remote command failed")
        || lower.contains("failed to solve")
        || message.len() > 240
    {
        return "We could not finish making your app. Wait a moment, then try again.".into();
    }
    message.to_string()
}

fn shown_email(value: &str) -> String {
    if acceptable_email(value) {
        value.to_string()
    } else {
        String::new()
    }
}

fn effective_from_email(value: &str) -> String {
    let value = value.trim().to_lowercase();
    if value.is_empty() || value == "onboarding@resend.dev" {
        DEFAULT_FROM_EMAIL.to_string()
    } else {
        value
    }
}

fn acceptable_email(value: &str) -> bool {
    valid_sign_in_email(value)
}

fn encryption_key_for_app(current: &str, new_app: bool) -> String {
    if !new_app && !current.trim().is_empty() {
        current.to_string()
    } else {
        random_token(32)
    }
}

fn valid_sign_in_email(value: &str) -> bool {
    let Some((name, domain)) = value.split_once('@') else {
        return false;
    };
    let Some((host, ending)) = domain.rsplit_once('.') else {
        return false;
    };
    !name.is_empty()
        && !host.is_empty()
        && ending.len() >= 2
        && value.len() <= 200
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && name.chars().all(|character| character.is_ascii())
        && domain
            .chars()
            .all(|character| character.is_ascii() && character != '@')
}

fn valid_region(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && value.len() <= 32
        && value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
}

fn require_ip(value: &str) -> Result<IpAddr, String> {
    value
        .parse()
        .map_err(|_| format!("{value} is not a usable IP address."))
}

fn ssh_destination(ip: &str) -> Result<String, String> {
    match require_ip(ip)? {
        IpAddr::V4(address) => Ok(format!("root@{address}")),
        IpAddr::V6(address) => Ok(format!("root@[{address}]")),
    }
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|err| err.to_string())?;
        file.write_all(bytes).map_err(|err| err.to_string())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, bytes).map_err(|err| err.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_read_writes_an_empty_configuration() {
        let dir = std::env::temp_dir().join(format!("shell-deploy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config = ensure_config(&dir).unwrap();
        assert!(config.digital_ocean_token.is_empty());
        assert!(config.resend_api_key.is_empty());
        assert!(config.public_ip.is_empty());
        assert_eq!(config.region, "nyc3");
        assert!(config_path(&dir).is_file());
        let again = ensure_config(&dir).unwrap();
        assert_eq!(again, config);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_window_does_not_receive_saved_secrets() {
        let mut config = DeploymentConfig::default();
        config.digital_ocean_token = "dop_v1_secret".into();
        config.resend_api_key = "re_secret".into();
        config.ssh_private_key = "private-key".into();
        config.db_password = "db-secret".into();
        config.encryption_key = "enc-secret".into();
        config.secret_key = "jwt-secret".into();
        config.admin_password = "admin-secret".into();
        config.public_ip = "203.0.113.10".into();
        let view = view(config, "", false);
        let json = serde_json::to_string(&view).unwrap();
        assert!(view.has_digital_ocean_token);
        assert!(view.has_resend_api_key);
        assert_eq!(view.admin_password, "admin-secret");
        assert!(!json.contains("dop_v1_secret"));
        assert!(!json.contains("re_secret"));
        assert!(!json.contains("private-key"));
        assert!(!json.contains("db-secret"));
        assert!(!json.contains("enc-secret"));
        assert!(!json.contains("jwt-secret"));
    }

    #[test]
    fn an_empty_token_does_not_erase_the_saved_one() {
        let dir = std::env::temp_dir().join(format!("shell-deploy-keep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        save_input(
            &dir,
            DeploymentInput {
                digital_ocean_token: "dop_v1_secret".into(),
                resend_api_key: String::new(),
                from_email: String::new(),
                admin_email: String::new(),
                region: "nyc3".into(),
                size: "ignored".into(),
            },
        )
        .unwrap();
        let kept = save_input(
            &dir,
            DeploymentInput {
                digital_ocean_token: String::new(),
                resend_api_key: "re_secret".into(),
                from_email: String::new(),
                admin_email: String::new(),
                region: "sfo3".into(),
                size: String::new(),
            },
        )
        .unwrap();
        assert_eq!(kept.digital_ocean_token, "dop_v1_secret");
        assert_eq!(kept.resend_api_key, "re_secret");
        assert_eq!(kept.region, "sfo3");
        assert_eq!(kept.size, DROPLET_SIZE);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sign_in_email_is_saved_and_a_blank_one_does_not_erase_it() {
        let dir = std::env::temp_dir().join(format!("shell-deploy-email-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let saved = save_input(
            &dir,
            DeploymentInput {
                digital_ocean_token: String::new(),
                resend_api_key: String::new(),
                from_email: String::new(),
                admin_email: "Owner@Example.com".into(),
                region: String::new(),
                size: String::new(),
            },
        )
        .unwrap();
        assert_eq!(saved.admin_email, "owner@example.com");
        assert_eq!(
            view(saved.clone(), "", false).admin_email,
            "owner@example.com"
        );
        let kept = save_input(
            &dir,
            DeploymentInput {
                digital_ocean_token: String::new(),
                resend_api_key: String::new(),
                from_email: String::new(),
                admin_email: String::new(),
                region: String::new(),
                size: String::new(),
            },
        )
        .unwrap();
        assert_eq!(kept.admin_email, "owner@example.com");
        let rejected = save_input(
            &dir,
            DeploymentInput {
                digital_ocean_token: String::new(),
                resend_api_key: String::new(),
                from_email: String::new(),
                admin_email: "admin@localhost".into(),
                region: String::new(),
                size: String::new(),
            },
        );
        assert!(rejected.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sending_address_is_saved_and_the_placeholder_is_refused() {
        let dir = std::env::temp_dir().join(format!("shell-deploy-from-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let saved = save_input(
            &dir,
            DeploymentInput {
                digital_ocean_token: String::new(),
                resend_api_key: String::new(),
                from_email: "Notes@Example.com".into(),
                admin_email: String::new(),
                region: String::new(),
                size: String::new(),
            },
        )
        .unwrap();
        assert_eq!(saved.from_email, "notes@example.com");
        assert_eq!(view(saved, "", false).from_email, "notes@example.com");
        assert_eq!(
            view(DeploymentConfig::default(), "", false).from_email,
            DEFAULT_FROM_EMAIL
        );
        let replaced = save_input(
            &dir,
            DeploymentInput {
                digital_ocean_token: String::new(),
                resend_api_key: String::new(),
                from_email: "onboarding@resend.dev".into(),
                admin_email: String::new(),
                region: String::new(),
                size: String::new(),
            },
        )
        .unwrap();
        assert_eq!(replaced.from_email, DEFAULT_FROM_EMAIL);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_new_app_gets_its_own_encryption_key() {
        let first = encryption_key_for_app("", true);
        let second = encryption_key_for_app("", true);
        assert_ne!(first, second);
        assert_eq!(
            encryption_key_for_app("kept-key-for-this-app", false),
            "kept-key-for-this-app"
        );
        assert_ne!(
            encryption_key_for_app("kept-key-for-this-app", true),
            "kept-key-for-this-app"
        );
    }

    #[test]
    fn an_unavailable_place_is_explained_in_plain_language() {
        let message =
            plain_failure("DigitalOcean returned 422: Size is not available in this region.");
        assert!(message.contains("Choose another place"));
        assert!(!message.contains("422"));
        assert!(!region_is_offered(&json!({
            "slug": "nyc1",
            "available": true,
            "sizes": ["s-1vcpu-1gb"]
        })));
        assert!(region_is_offered(&json!({
            "slug": "nyc3",
            "available": true,
            "sizes": ["s-2vcpu-4gb"]
        })));
    }

    #[test]
    fn the_highest_numbered_tag_is_chosen() {
        let tags = vec![
            "latest".into(),
            "7.9.0".into(),
            "7.10.0".into(),
            "7.3.0".into(),
            "main".into(),
        ];
        assert_eq!(highest_version(&tags).as_deref(), Some("7.10.0"));
        let compose = pinned_compose("7.3.0", "1.0.0");
        assert!(compose.contains("techfa22/fourss-api:7.3.0"));
        assert!(compose.contains("techfa22/fourss-ui:1.0.0"));
        assert!(!compose.contains("techfa22/fourss-api:latest"));
        assert!(!compose.contains("techfa22/fourss-ui:latest"));
    }

    #[test]
    fn a_finished_setup_can_be_copied_somewhere_else() {
        let dir = std::env::temp_dir().join(format!("shell-deploy-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut config = ensure_config(&dir).unwrap();
        config.public_ip = "203.0.113.10".into();
        config.admin_password = "sign-in-secret".into();
        write_config(&config_path(&dir), &config).unwrap();
        let destination = dir.join("copy.json");
        export_configuration(&dir, &destination).unwrap();
        let copy = fs::read_to_string(&destination).unwrap();
        assert!(copy.contains("sign-in-secret"));
        assert!(export_configuration(&dir, &config_path(&dir)).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
