//! A lightweight GitHub API client
use bytes::Bytes;
use std::{string::FromUtf8Error, sync::Arc};

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const GITHUB_API_URL: &str = "https://api.github.com";

/// Error variants that we can encounter when making requests to the GitHub REST API
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No GitHub API client available
    #[error("no GitHub client available")]
    NoClient,

    // Wrapped errors
    /// Invalid utf-8 found while trying to decode file content from GitHub
    #[error(transparent)]
    InvalidUtf8(#[from] FromUtf8Error),

    /// An underlying error from the reqwest crate
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

/// An API client that can make requests to the GitHub REST API.
#[allow(async_fn_in_trait)]
pub trait Client {
    /// Attempt to pull the raw file content of a given file as [Bytes] from the specified GitHub
    /// repo.
    ///
    /// The API token used to create this client must have access to the repo in question.
    async fn raw_file_content(
        &self,
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<impl AsRef<str>>,
    ) -> Result<Bytes, Error>;

    /// Attempt to pull the raw file content of a given file as a utf-8 [String] from the specified
    /// GitHub repo.
    ///
    /// The API token used to create this client must have access to the repo in question.
    async fn string_file_content(
        &self,
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<impl AsRef<str>>,
    ) -> Result<String, Error> {
        let bytes = self.raw_file_content(org, repo, path, git_ref).await?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }
}

/// A lightweight GitHub API client for the subset of REST endpoints we need to work with.
///
/// This client makes use of bearer auth as documented for the GitHub API here:
///   <https://docs.github.com/en/rest/authentication/authenticating-to-the-rest-api?apiVersion=2022-11-28>
///
/// Creation of access tokens is documented here:
///   <https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#types-of-personal-access-tokens>
#[derive(Clone, Debug)]
pub struct GithubClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) base_url: Arc<str>,
    pub(crate) api_token: Arc<str>,
}

impl GithubClient {
    /// Construct a new client with a dedicated underlying [reqwest::Client].
    pub fn new(api_token: impl Into<String>) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base_url: GITHUB_API_URL.into(),
            api_token: api_token.into().into(),
        }
    }

    /// Construct a new client with a dedicated underlying [reqwest::Client] for the provided base
    /// URL.
    pub fn new_with_base_url(base_url: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base_url: base_url.into().into(),
            api_token: api_token.into().into(),
        }
    }
}

impl Client for GithubClient {
    async fn raw_file_content(
        &self,
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<impl AsRef<str>>,
    ) -> Result<Bytes, Error> {
        let mut url = format!("{}/repos/{org}/{repo}/contents/{path}", self.base_url);
        if let Some(r) = git_ref {
            url = format!("{url}?ref={}", r.as_ref())
        }

        let res = self
            .inner
            .get(url)
            .bearer_auth(&self.api_token)
            .header("User-Agent", format!("apollo-rtf-{PKG_VERSION}"))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("accept", "application/vnd.github.v3.raw")
            .send()
            .await?
            .error_for_status()?;

        Ok(res.bytes().await?)
    }
}
