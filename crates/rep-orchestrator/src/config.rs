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
    pub db_pass: String,
    pub github_token: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_executions: usize,
    #[serde(default = "default_max_queued")]
    pub max_queued_executions: usize,
    pub kubeconfig_path: String,
    pub mgmt_context: String,
    pub workload_context: String,
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
