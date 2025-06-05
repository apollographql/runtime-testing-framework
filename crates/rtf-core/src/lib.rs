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
#[derive(Debug, Default)]
pub struct ReqwestClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) platform: Option<PlatformConfig>,
}

impl ReqwestClient {
    /// Construct a new [ReqwestClient]
    pub fn new() -> Self {
        Self {
            inner: reqwest::Client::new(),
            platform: None,
        }
    }

    /// Provide configuration for making requests to the Apollo platform API.
    pub fn with_platform_config(&mut self, api_key: impl Into<String>, staging: bool) -> &mut Self {
        let url = if staging {
            STAGING_STUDIO_URL
        } else {
            PROD_STUDIO_URL
        };

        self.platform = Some(PlatformConfig {
            url: url.into(),
            api_key: api_key.into(),
        });

        self
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
