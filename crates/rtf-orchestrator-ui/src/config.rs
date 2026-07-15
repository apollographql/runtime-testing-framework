use serde::Deserialize;
use std::{net::SocketAddr, sync::LazyLock};

static CONFIG: LazyLock<Config> =
    LazyLock::new(|| match envy::prefixed("RTF_UI_").from_env::<Config>() {
        Ok(cfg) => cfg,
        Err(e) => panic!("unable to load config from env: {e}"),
    });

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub orchestrator_url: String,
}

impl Config {
    pub fn get() -> &'static Self {
        &CONFIG
    }

    pub fn socket_addr(&self) -> SocketAddr {
        match format!("{}:{}", self.host, self.port).parse() {
            Ok(sa) => sa,
            Err(e) => panic!("invalid socket addr from config: {e}"),
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".to_owned()
}

fn default_port() -> u16 {
    8080
}
