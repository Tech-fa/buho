use serde::Serialize;

use crate::deploy::{self, DeploymentView};

pub const FOURSS_PRODUCT_ID: &str = "fourss";
pub const FOURSS_INSTANCE_ID: &str = "fourss-default";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    NotInstalled,
    Deploying,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppInstance {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub status: InstanceStatus,
    pub app_url: Option<String>,
    pub api_url: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub products: Vec<Product>,
    pub instances: Vec<AppInstance>,
}

pub fn fourss_product() -> Product {
    Product {
        id: FOURSS_PRODUCT_ID.into(),
        name: "Fourss".into(),
        summary: "Quality and audit workspace.".into(),
        provider: "fourss-deploy".into(),
    }
}

pub fn from_deployment(view: &DeploymentView) -> CatalogView {
    CatalogView {
        products: vec![fourss_product()],
        instances: vec![instance_from(view)],
    }
}

fn instance_from(view: &DeploymentView) -> AppInstance {
    let status = if view.running {
        InstanceStatus::Deploying
    } else if !view.app_url.is_empty() {
        InstanceStatus::Running
    } else if !view.status.is_empty() && view.status != "Your app is ready." {
        InstanceStatus::Error
    } else {
        InstanceStatus::NotInstalled
    };
    let api_url = if view.https_ready && !view.api_host.is_empty() {
        Some(format!("https://{}", view.api_host))
    } else if !view.public_ip.is_empty() {
        Some(format!("http://{}:3012", view.public_ip))
    } else {
        None
    };
    AppInstance {
        id: FOURSS_INSTANCE_ID.into(),
        product_id: FOURSS_PRODUCT_ID.into(),
        name: "Fourss".into(),
        status,
        app_url: nonempty(&view.app_url),
        api_url,
        message: view.status.clone(),
    }
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn catalog_view(
    data_dir: &std::path::Path,
    status: &str,
    running: bool,
) -> Result<CatalogView, String> {
    let config = deploy::ensure_config(data_dir)?;
    Ok(from_deployment(&deploy::view(config, status, running)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::DeploymentView;

    fn blank_view() -> DeploymentView {
        DeploymentView {
            has_digital_ocean_token: false,
            has_resend_api_key: false,
            region: "nyc3".into(),
            size: "s-2vcpu-4gb".into(),
            droplet_id: 0,
            droplet_name: String::new(),
            public_ip: String::new(),
            from_email: String::new(),
            admin_email: String::new(),
            admin_password: String::new(),
            status: String::new(),
            running: false,
            app_url: String::new(),
            api_version: String::new(),
            ui_version: String::new(),
            domain: String::new(),
            app_host: String::new(),
            api_host: String::new(),
            https_ready: false,
            host: "digitalocean".into(),
            ssh_user: String::new(),
            has_ssh_key: false,
            installed: false,
        }
    }

    #[test]
    fn empty_config_is_not_installed() {
        let view = from_deployment(&blank_view());
        assert_eq!(view.products.len(), 1);
        assert_eq!(view.products[0].id, "fourss");
        assert_eq!(view.instances.len(), 1);
        assert_eq!(view.instances[0].id, "fourss-default");
        assert_eq!(view.instances[0].status, InstanceStatus::NotInstalled);
    }

    #[test]
    fn an_app_url_means_running() {
        let mut view = blank_view();
        view.app_url = "http://203.0.113.10".into();
        view.public_ip = "203.0.113.10".into();
        let catalog = from_deployment(&view);
        assert_eq!(catalog.instances[0].status, InstanceStatus::Running);
        assert_eq!(
            catalog.instances[0].app_url.as_deref(),
            Some("http://203.0.113.10")
        );
        assert_eq!(
            catalog.instances[0].api_url.as_deref(),
            Some("http://203.0.113.10:3012")
        );
    }

    #[test]
    fn a_running_deploy_is_deploying() {
        let mut view = blank_view();
        view.running = true;
        view.app_url = "http://203.0.113.10".into();
        assert_eq!(
            from_deployment(&view).instances[0].status,
            InstanceStatus::Deploying
        );
    }

    #[test]
    fn a_status_without_a_url_is_an_error() {
        let mut view = blank_view();
        view.status = "We could not finish making your app.".into();
        assert_eq!(
            from_deployment(&view).instances[0].status,
            InstanceStatus::Error
        );
        assert_eq!(
            from_deployment(&view).instances[0].message,
            "We could not finish making your app."
        );
    }

    #[test]
    fn catalog_view_reads_an_empty_data_dir() {
        let dir = std::env::temp_dir().join(format!(
            "buho-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let view = catalog_view(&dir, "", false).unwrap();
        assert_eq!(view.instances[0].status, InstanceStatus::NotInstalled);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
