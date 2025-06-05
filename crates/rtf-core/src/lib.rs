//! The core functionality of the Apollo Runtime Testing Framework.
#![warn(
    clippy::complexity,
    clippy::correctness,
    clippy::style,
    future_incompatible,
    missing_debug_implementations,
    missing_docs,
    rust_2018_idioms,
    rustdoc::all
)]
#![deny(clippy::undocumented_unsafe_blocks)]
use std::fmt;

pub mod platform_query;
pub mod supergraph;

use platform_query::{PROD_STUDIO_URL, STAGING_STUDIO_URL};

/// The maximum number of queries to run in parallel querying the platform API.
pub const N_PARALLEL_FETCH: usize = 20;

/// A client implementation that is backed by a [reqwest::Client].
#[derive(Debug)]
pub struct ReqwestClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) platform: PlatformConfig,
}

impl ReqwestClient {
    /// Construct a new [ReqwestClient] with the provided config
    pub fn new(platform: PlatformConfig) -> Self {
        Self {
            inner: reqwest::Client::new(),
            platform,
        }
    }

    /// Construct a new [ReqwestClient] for interacting with the staging studio API
    pub fn new_staging(api_key: impl Into<String>) -> Self {
        Self::new(PlatformConfig::new(STAGING_STUDIO_URL, api_key))
    }

    /// Construct a new [ReqwestClient] for interacting with the production studio API
    pub fn new_prod(api_key: impl Into<String>) -> Self {
        Self::new(PlatformConfig::new(PROD_STUDIO_URL, api_key))
    }
}

/// Configuration for making requests to the Apollo platform API
pub struct PlatformConfig {
    pub(crate) url: String,
    pub(crate) api_key: String,
}

impl PlatformConfig {
    /// Construct a new [PlatformConfig]
    pub fn new(url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            api_key: api_key.into(),
        }
    }
}

// Custom Debug impl to prevent us dumping the api key
impl fmt::Debug for PlatformConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformConfig")
            .field("url", &self.url)
            .finish()
    }
}
