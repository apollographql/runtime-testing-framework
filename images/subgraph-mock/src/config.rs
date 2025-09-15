use crate::latency::{LatencyConfig, LatencyGenerator};
use hyper::{
    HeaderMap,
    header::{HeaderName, HeaderValue},
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub latency: LatencyConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            headers: Default::default(),
            latency: Default::default(),
        }
    }
}

impl Config {
    pub fn into_parts(self) -> (u16, LatencyGenerator, HeaderMap<HeaderValue>) {
        let latency_generator = LatencyGenerator::new(self.latency);
        let additional_headers: HeaderMap<HeaderValue> = self
            .headers
            .into_iter()
            .map(|(k, v)| {
                (
                    HeaderName::try_from(&k)
                        .unwrap_or_else(|_| panic!("'{k}' is not a valid header name")),
                    HeaderValue::try_from(&v)
                        .unwrap_or_else(|_| panic!("'{v}' is not a valid header value")),
                )
            })
            .collect();

        (self.port, latency_generator, additional_headers)
    }
}

fn default_port() -> u16 {
    8080
}
