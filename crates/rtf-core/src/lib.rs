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

pub mod graphos;

use graphos::{
    PlatformClient,
    platform_query::{PROD_STUDIO_URL, STAGING_STUDIO_URL},
};

/// The maximum number of queries to run in parallel querying the platform API.
pub const N_PARALLEL_FETCH: usize = 20;

/// A client implementation that is backed by a [reqwest::Client].
#[derive(Debug, Default)]
pub struct ReqwestClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) platform: Option<PlatformClient>,
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

        self.platform = Some(PlatformClient {
            inner: self.inner.clone(),
            url: url.into(),
            api_key: api_key.into(),
        });

        self
    }

    /// Obtain a reference to an API client for running operations with the Apollo platform API if
    /// config is available.
    pub fn platform_client(&self) -> Option<&PlatformClient> {
        self.platform.as_ref()
    }
}
