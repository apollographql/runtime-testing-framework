use rtf_config::context::Context;
use serde::Deserialize;
use std::{net::SocketAddr, sync::LazyLock};
use tracing::warn;

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
    #[serde(default = "default_body_limit_mb")]
    pub body_limit_mb: usize,
    pub kubeconfig_path: String,
    #[serde(default = "default_kubeconfig_secret_name")]
    pub kubeconfig_secret_name: String,
    pub admins_path: String,
    pub workload_context: String,
    pub orchestrator_url: String,
    pub toolbox_pull_policy: String,
    pub otel_collector_grpc: String,
    pub otel_collector_http: String,
    pub prometheus_endpoint: String,
    pub gcs_bucket: String,
    #[serde(default = "default_gcs_url_ttl_secs")]
    pub gcs_url_ttl_secs: u64,
    #[serde(default = "default_failed_execution_ttl_secs")]
    pub failed_execution_ttl_secs: u64,
    #[serde(default = "default_retry_window_secs")]
    pub retry_window_secs: u64,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
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

    pub fn server_context(&self) -> Context {
        let mut ctx = Context::new();
        // apollo_sudo always true; graphos_staging always false for the orchestrator
        ctx.with_platform_config(&self.apollo_key, false, true);
        ctx.with_github_app_config(self.github_app_id, self.github_app_private_key_pem.clone());

        ctx
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

fn default_body_limit_mb() -> usize {
    50
}

fn default_gcs_url_ttl_secs() -> u64 {
    300
}

fn default_kubeconfig_secret_name() -> String {
    warn!("RTF_KUBECONFIG_SECRET_NAME not set. Using default value for local cluster");

    "workload-kubeconfig".to_string()
}

fn default_failed_execution_ttl_secs() -> u64 {
    600
}

fn default_retry_window_secs() -> u64 {
    5 * 60
}

fn default_poll_interval_secs() -> u64 {
    10
}
