use crate::{
    handle::graphql::ResponseGenerationConfig,
    latency::{LatencyConfig, LatencyGenerator},
};
use hyper::{
    HeaderMap,
    header::{HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub latency: LatencyConfig,
    #[serde(default)]
    pub response_generation: ResponseGenerationConfig,
    #[serde(default = "default_cache_responses")]
    pub cache_responses: bool,
}

fn default_port() -> u16 {
    8080
}

fn default_cache_responses() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            headers: Default::default(),
            latency: Default::default(),
            response_generation: Default::default(),
            cache_responses: default_cache_responses(),
        }
    }
}

impl Config {
    pub fn into_parts(
        self,
    ) -> (
        u16,
        bool,
        LatencyGenerator,
        HeaderMap<HeaderValue>,
        ResponseGenerationConfig,
    ) {
        info!(config=%serde_json::to_string(&self.latency).unwrap(), "latency generation");
        let latency_generator = LatencyGenerator::new(self.latency);

        info!(headers=%serde_json::to_string(&self.headers).unwrap(), "additional headers");
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

        let mut response_generation = self.response_generation;
        let mut scalars = ResponseGenerationConfig::default().scalars;
        scalars.extend(response_generation.scalars);
        response_generation.scalars = scalars;

        info!(config=%serde_json::to_string(&response_generation).unwrap(), "response generation");

        (
            self.port,
            self.cache_responses,
            latency_generator,
            additional_headers,
            response_generation,
        )
    }
}
