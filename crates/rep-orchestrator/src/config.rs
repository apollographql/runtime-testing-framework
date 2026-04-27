use serde::Deserialize;
use std::{net::SocketAddr, sync::LazyLock};

static CONFIG: LazyLock<Config> =
    LazyLock::new(|| match envy::prefixed("RTF_").from_env::<Config>() {
        Ok(cfg) => cfg,
        Err(e) => panic!("unable to load config from env: {e}"),
    });

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub apollo_key: String,
    pub db_host: String,
    pub db_port: u16,
    pub db_name: String,
    pub db_user: String,
    #[serde(default)]
    pub db_pass: Option<String>,
    pub github_app_id: u64,
    pub github_app_private_key_pem: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_executions: usize,
    #[serde(default = "default_max_queued")]
    pub max_queued_executions: usize,
    pub kubeconfig_path: String,
    #[serde(default)]
    pub mgmt_context: Option<String>,
    pub workload_context: String,
    pub orchestrator_url: String,
    pub gcs_bucket: String,
    #[serde(default = "default_gcs_url_ttl_secs")]
    pub gcs_url_ttl_secs: u64,
    #[serde(default)]
    pub mock_internal_gcs_url: Option<String>,
    #[serde(default)]
    pub mock_public_gcs_url: Option<String>,
}

impl Config {
    pub fn get() -> &'static Self {
        &CONFIG
    }

    pub fn socket_addr(&self) -> SocketAddr {
        match format!("{}:{}", self.host, self.port).parse() {
            Ok(sa) => sa,
            Err(e) => panic!("invalid socker addr from config: {e}"),
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".to_owned()
}

fn default_port() -> u16 {
    8035
}

fn default_max_concurrent() -> usize {
    10
}

fn default_max_queued() -> usize {
    100
}

fn default_gcs_url_ttl_secs() -> u64 {
    300
}
