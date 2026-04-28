//! The integrations with external APIs for the Apollo Runtime Testing Framework.
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
use github::GithubClient;
use reqwest::{Error, StatusCode};
use std::{collections::HashMap, future::Future};

pub mod github;
pub mod graphos;

use graphos::{
    PlatformClient,
    platform_query::{PROD_STUDIO_URL, STAGING_STUDIO_URL},
};

/// The name reserved for the "default" platform environment — the production GraphOS instance,
/// where customer graphs live.
pub const DEFAULT_GRAPHOS_ENV_NAME: &str = "default";

/// The environment variable name for the api key used to authenticate with the GraphOS API
pub const APOLLO_KEY_ENV_VAR: &str = "APOLLO_KEY";
/// The environment variable name for setting the apollo-sudo=true header in GraphOS API requests
pub const APOLLO_SUDO_ENV_VAR: &str = "APOLLO_SUDO";
/// The maximum number of queries to run in parallel querying the platform API.
pub const N_PARALLEL_FETCH: usize = 20;
/// The environment variable name for the api token used to authenticate with the GitHub API.
pub const GITHUB_TOKEN_ENV_VAR: &str = "GITHUB_TOKEN";

/// Static metadata for a GraphOS environment that rtf knows how to talk to.
///
/// The set of environments rtf supports is fixed at build time via [KNOWN_GRAPHOS_ENVS] — test
/// plans select one by name through the `graphos_env` field on a GraphOS file provider, and
/// rtf reads the corresponding [Self::api_key_env_var] from its process environment to obtain
/// the credential.
///
/// Adding a new environment is a code change here (a new entry in [KNOWN_GRAPHOS_ENVS] plus a
/// URL constant in `graphos::platform_query`).  Keeping the set closed and static is what lets
/// the rep-orchestrator provision the right secrets at deploy time without coordinating with
/// arbitrary plan-author-declared env var names.
#[derive(Debug, Clone, Copy)]
pub struct KnownGraphosEnv {
    /// The name used to reference this environment from `graphos_env` fields on file providers.
    pub name: &'static str,
    /// The GraphOS API endpoint for this environment.
    pub url: &'static str,
    /// The env var the rtf process reads to obtain this environment's API key.
    pub api_key_env_var: &'static str,
    /// Whether requests routed to this environment include the `apollo-sudo: true` header.
    pub sudo: bool,
}

/// Every GraphOS environment that rtf recognises.  See [KnownGraphosEnv] for how this list is
/// expected to grow.
pub const KNOWN_GRAPHOS_ENVS: &[KnownGraphosEnv] = &[
    KnownGraphosEnv {
        name: DEFAULT_GRAPHOS_ENV_NAME,
        url: PROD_STUDIO_URL,
        api_key_env_var: APOLLO_KEY_ENV_VAR,
        sudo: false,
    },
    KnownGraphosEnv {
        name: "staging",
        url: STAGING_STUDIO_URL,
        api_key_env_var: "APOLLO_KEY_STAGING",
        sudo: true,
    },
];

/// Look up a known GraphOS environment by name.
pub fn known_graphos_env(name: &str) -> Option<&'static KnownGraphosEnv> {
    KNOWN_GRAPHOS_ENVS.iter().find(|env| env.name == name)
}

/// A client implementation that is backed by a [reqwest::Client].
#[derive(Debug, Default, Clone)]
pub struct ReqwestClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) github: Option<GithubClient>,
    pub(crate) platforms: HashMap<String, PlatformClient>,
}

impl ReqwestClient {
    /// Construct a new [ReqwestClient]
    pub fn new() -> Self {
        Self {
            inner: reqwest::Client::new(),
            github: None,
            platforms: HashMap::new(),
        }
    }

    /// Obtain a reference to the default platform client if it has been configured.
    ///
    /// This is a convenience for [ReqwestClient::platform_client_for] with the
    /// [DEFAULT_GRAPHOS_ENV_NAME] environment name.
    pub fn platform_client(&self) -> Option<&PlatformClient> {
        self.platform_client_for(DEFAULT_GRAPHOS_ENV_NAME)
    }

    /// Obtain a reference to a named platform client if it has been configured.
    pub fn platform_client_for(&self, env_name: &str) -> Option<&PlatformClient> {
        self.platforms.get(env_name)
    }

    /// Obtain a reference to an API client for making requests to the GitHub REST API if config is
    /// available.
    pub fn github_client(&self) -> Option<&GithubClient> {
        self.github.as_ref()
    }

    /// Register a named platform environment.
    ///
    /// `env_name` is the key that test plans use when referencing this environment via the
    /// `graphos_env` field on a GraphOS file provider (e.g. `"default"`, `"apollo_staging"`).
    /// `url` is the GraphOS API endpoint for this environment.
    /// `api_key` is the credential sent as the `x-api-key` header.
    /// If `sudo` is true, requests include the `apollo-sudo: true` header.
    pub fn with_platform_env(
        &mut self,
        env_name: impl Into<String>,
        url: impl Into<String>,
        api_key: impl Into<String>,
        sudo: bool,
    ) -> &mut Self {
        self.platforms.insert(
            env_name.into(),
            PlatformClient {
                inner: self.inner.clone(),
                url: url.into().into(),
                api_key: api_key.into().into(),
                sudo,
            },
        );

        self
    }

    /// Provide configuration for GitHub App installation authentication.
    ///
    /// `private_key_pem` must be the PKCS#1 PEM private key generated by GitHub
    /// ("BEGIN RSA PRIVATE KEY"). Installation IDs are discovered per-org on first use; access
    /// tokens are cached and refreshed automatically.
    pub fn with_github_app_config(
        &mut self,
        app_id: u64,
        private_key_pem: impl Into<String>,
    ) -> &mut Self {
        self.github = Some(GithubClient::new_from_app(app_id, private_key_pem));
        self
    }

    /// Provide configuration for making requests to the GitHub REST API.
    pub fn with_github_config(&mut self, api_token: impl Into<String>) -> &mut Self {
        self.github = Some(GithubClient::from_shared_client(
            self.inner.clone(),
            api_token,
        ));

        self
    }
}

/// Represents the result of an HTTP request made by an `HttpClient`.
///
/// Contains the HTTP status code and the raw response body as bytes.
#[derive(Debug, Clone)]
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
pub trait HttpClient: Send + Sync {
    /// Sends an HTTP GET request to the given `url` and returns the full response.
    ///
    /// This method performs no automatic error handling or status code validation;
    /// callers are responsible for interpreting the response body, including parsing
    /// it as JSON or text if desired, and handling any HTTP status errors.
    fn get(&self, url: &str) -> impl Future<Output = Result<HttpResponse, Error>> + Send;
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
