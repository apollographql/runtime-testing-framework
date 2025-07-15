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

use bytes::Bytes;
use reqwest::{Error, StatusCode};

pub mod github;
pub mod graphos;

use graphos::{
    PlatformClient,
    platform_query::{PROD_STUDIO_URL, STAGING_STUDIO_URL},
};

/// The environment variable name for the api key used to authenticate with the GraphOS API
pub const APOLLO_KEY_ENV_VAR: &str = "APOLLO_KEY";
/// The environment variable name for setting the apollo-sudo=true header in GraphOS API requests
pub const APOLLO_SUDO_ENV_VAR: &str = "APOLLO_SUDO";
/// The environment variable name for whether or not to use the staging GraphOS API
pub const GRAPH_OS_STAGING_ENV_VAR: &str = "GRAPHOS_STAGING";
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
    pub fn with_platform_config(
        &mut self,
        api_key: impl Into<String>,
        staging: bool,
        sudo: bool,
    ) -> &mut Self {
        let url = if staging {
            STAGING_STUDIO_URL
        } else {
            PROD_STUDIO_URL
        };

        self.platform = Some(PlatformClient {
            inner: self.inner.clone(),
            url: url.into(),
            api_key: api_key.into(),
            sudo,
        });

        self
    }

    /// Obtain a reference to an API client for running operations with the Apollo platform API if
    /// config is available.
    pub fn platform_client(&self) -> Option<&PlatformClient> {
        self.platform.as_ref()
    }
}

/// Represents the result of an HTTP request made by an `HttpClient`.
///
/// Contains the HTTP status code and the raw response body as bytes.
#[derive(Debug)]
pub struct HttpResponse {
    /// The HTTP status code returned by the server (e.g., 200 OK, 404 Not Found).
    pub status: StatusCode,

    /// The raw response body returned by the server, as bytes.
    ///
    /// This may contain any type of content (JSON, text, binary, etc.),
    /// and should be interpreted by the caller as needed.
    pub body: Bytes,
}

/// Types that implement HttpClient may be used to perform http requests
#[allow(async_fn_in_trait)]
pub trait HttpClient {
    /// Sends an HTTP GET request to the given `url` and returns the full response.
    ///
    /// This method performs no automatic error handling or status code validation;
    /// callers are responsible for interpreting the response body, including parsing
    /// it as JSON or text if desired, and handling any HTTP status errors.
    async fn get(&self, url: &str) -> Result<HttpResponse, Error>;
}

impl HttpClient for ReqwestClient {
    async fn get(&self, url: &str) -> Result<HttpResponse, Error> {
        let response = self.inner.get(url).send().await?;
        Ok(HttpResponse {
            status: response.status(),
            body: response.bytes().await?,
        })
    }
}
