//! A lightweight GitHub API client
use bytes::Bytes;
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    string::FromUtf8Error,
    sync::{Arc, Mutex},
};

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const GITHUB_API_URL: &str = "https://api.github.com";

/// Error variants that we can encounter when making requests to the GitHub REST API
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No GitHub API client available
    #[error("no GitHub client available")]
    NoClient,

    /// Failed to sign a GitHub App JWT
    #[error("failed to sign GitHub App JWT: {0}")]
    JwtEncoding(#[source] jsonwebtoken::errors::Error),

    /// Failed to discover the GitHub App installation for an organization
    #[error("failed to discover GitHub App installation for org '{org}': {source}")]
    InstallationDiscovery {
        /// The org slug for which discovery failed
        org: String,
        /// The underlying HTTP error
        #[source]
        source: reqwest::Error,
    },

    /// Invalid rsa key
    #[error("invalid RSA key: {0}")]
    InvalidRsaKey(#[source] jsonwebtoken::errors::Error),

    /// Failed to acquire a GitHub App installation access token
    #[error("failed to acquire GitHub App installation access token: {0}")]
    TokenAcquisition(#[source] reqwest::Error),

    /// GitHub App installation token response was malformed
    #[error("GitHub App installation token response was malformed")]
    MalformedTokenResponse,

    // Wrapped errors
    /// Invalid utf-8 found while trying to decode file content from GitHub
    #[error(transparent)]
    InvalidUtf8(#[from] FromUtf8Error),

    /// An underlying error from the reqwest crate
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

/// An API client that can make requests to the GitHub REST API.
pub trait Client: Send + Sync {
    /// Attempt to pull the raw file content of a given file as [Bytes] from the specified GitHub
    /// repo.
    ///
    /// The API token used to create this client must have access to the repo in question.
    fn raw_file_content<G: AsRef<str> + Send>(
        &self,
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<G>,
    ) -> impl Future<Output = Result<Bytes, Error>> + Send;

    /// Attempt to pull the raw file content of a given file as a utf-8 [String] from the specified
    /// GitHub repo.
    ///
    /// The API token used to create this client must have access to the repo in question.
    fn string_file_content<G: AsRef<str> + Send>(
        &self,
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<G>,
    ) -> impl Future<Output = Result<String, Error>> + Send {
        async move {
            let bytes = self.raw_file_content(org, repo, path, git_ref).await?;
            Ok(String::from_utf8(bytes.to_vec())?)
        }
    }

    /// Resolve a git ref (branch, tag, or SHA) to the commit SHA it points at. [None] resolves the
    /// repo's default branch.
    fn commit_sha<G: AsRef<str> + Send>(
        &self,
        org: &str,
        repo: &str,
        git_ref: Option<G>,
    ) -> impl Future<Output = Result<String, Error>> + Send;
}

fn commit_url(base_url: &str, org: &str, repo: &str, git_ref: Option<&str>) -> String {
    let git_ref = git_ref.unwrap_or("HEAD");

    format!("{base_url}/repos/{org}/{repo}/commits/{git_ref}")
}

/// A lightweight GitHub API client for the subset of REST endpoints we need to work with.
///
/// Supports two authentication modes:
///
/// - **Personal access token** via [`GithubClient::new`]: bearer auth with a static token.
/// - **GitHub App** via [`GithubClient::new_from_app`]: signs a short-lived JWT with the App's
///   private key. On the first request for a given org, discovers the installation ID via
///   `GET /orgs/{org}/installation`, then exchanges it for an installation access token. Both the
///   org→installation mapping and the access tokens are cached; tokens are refreshed automatically
///   on expiry.
///
/// Authentication is documented here:
///   <https://docs.github.com/en/rest/authentication/authenticating-to-the-rest-api?apiVersion=2022-11-28>
#[derive(Clone, Debug)]
pub struct GithubClient {
    pub(crate) inner: reqwest::Client,
    pub(crate) base_url: Arc<str>,
    auth: ApiToken,
}

impl GithubClient {
    /// Construct a new client authenticating with a personal access token.
    pub fn new(api_token: impl Into<String>) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base_url: GITHUB_API_URL.into(),
            auth: ApiToken::Static(api_token.into().into()),
        }
    }

    /// Construct a new client with a custom base URL, authenticating with a personal access token.
    pub fn new_with_base_url(base_url: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base_url: base_url.into().into(),
            auth: ApiToken::Static(api_token.into().into()),
        }
    }

    /// Construct a new client that authenticates using a GitHub App.
    ///
    /// `private_key_pem` must be the PKCS#1 PEM private key generated by GitHub
    /// ("BEGIN RSA PRIVATE KEY"). Installation IDs are discovered per-org on first use and cached
    /// for the lifetime of the client. Access tokens are cached per installation and refreshed
    /// automatically on expiry.
    pub fn new_from_app(app_id: u64, private_key_pem: impl Into<String>) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base_url: GITHUB_API_URL.into(),
            auth: ApiToken::AppInstallation(AppInstallationAuth {
                app_id,
                private_key_pem: private_key_pem.into().into(),
                installations: Arc::new(Mutex::new(HashMap::new())),
                cache: Arc::new(Mutex::new(HashMap::new())),
            }),
        }
    }

    /// Construct a PAT client that shares the provided [reqwest::Client] for connection pooling.
    pub(crate) fn from_shared_client(inner: reqwest::Client, api_token: impl Into<String>) -> Self {
        Self {
            inner,
            base_url: GITHUB_API_URL.into(),
            auth: ApiToken::Static(api_token.into().into()),
        }
    }

    async fn bearer_token(&self, org: &str) -> Result<Arc<str>, Error> {
        match &self.auth {
            ApiToken::Static(token) => Ok(Arc::clone(token)),
            ApiToken::AppInstallation(auth) => {
                auth.get_token(&self.inner, &self.base_url, org).await
            }
        }
    }
}

fn with_common_headers(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.header("User-Agent", format!("apollo-rtf-{PKG_VERSION}"))
        .header("X-GitHub-Api-Version", "2022-11-28")
}

impl Client for GithubClient {
    async fn raw_file_content<G: AsRef<str> + Send>(
        &self,
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<G>,
    ) -> Result<Bytes, Error> {
        let mut url = format!("{}/repos/{org}/{repo}/contents/{path}", self.base_url);
        if let Some(r) = git_ref {
            url = format!("{url}?ref={}", r.as_ref())
        }

        let token = self.bearer_token(org).await?;
        let res = with_common_headers(self.inner.get(url))
            .bearer_auth(token.as_ref())
            .header("accept", "application/vnd.github.v3.raw")
            .send()
            .await?
            .error_for_status()?;

        Ok(res.bytes().await?)
    }

    async fn commit_sha<G: AsRef<str> + Send>(
        &self,
        org: &str,
        repo: &str,
        git_ref: Option<G>,
    ) -> Result<String, Error> {
        let url = commit_url(
            &self.base_url,
            org,
            repo,
            git_ref.as_ref().map(|r| r.as_ref()),
        );

        let token = self.bearer_token(org).await?;
        let res = with_common_headers(self.inner.get(url))
            .bearer_auth(token.as_ref())
            // Returns the bare SHA as the body rather than the whole commit as JSON.
            .header("accept", "application/vnd.github.sha")
            .send()
            .await?
            .error_for_status()?;

        Ok(res.text().await?.trim().to_owned())
    }
}

#[derive(Clone)]
enum ApiToken {
    Static(Arc<str>),
    AppInstallation(AppInstallationAuth),
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static(_) => f.write_str("Static(...)"),
            Self::AppInstallation(auth) => {
                write!(f, "AppInstallation(app_id={})", auth.app_id)
            }
        }
    }
}

#[derive(Clone)]
struct AppInstallationAuth {
    app_id: u64,
    private_key_pem: Arc<str>,
    /// org slug → installation ID; populated on first request per org, never evicted.
    installations: Arc<Mutex<HashMap<String, u64>>>,
    /// installation ID → cached access token; shared across clones.
    cache: Arc<Mutex<HashMap<u64, CachedToken>>>,
}

#[derive(Clone, Debug)]
struct CachedToken {
    token: Arc<str>,
    expires_at: DateTime<Utc>,
}

impl AppInstallationAuth {
    pub(crate) fn sign_jwt(&self) -> Result<String, Error> {
        let now = Utc::now().timestamp();
        let claims = Claims {
            iat: now - 60,  // 60s in the past to absorb clock skew
            exp: now + 600, // 10 minutes, the GitHub maximum
            iss: self.app_id.to_string(),
        };

        let key = EncodingKey::from_rsa_pem(self.private_key_pem.as_bytes())
            .map_err(Error::InvalidRsaKey)?;

        return encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(Error::JwtEncoding);

        // Serde structs
        #[derive(Serialize)]
        struct Claims {
            iat: i64,
            exp: i64,
            iss: String,
        }
    }

    fn cached_token(&self, installation_id: u64) -> Option<Arc<str>> {
        let cache = self.cache.lock().expect("cache lock poisoned");
        cache
            .get(&installation_id)
            .filter(|c| c.expires_at > Utc::now())
            .map(|c| Arc::clone(&c.token))
    }

    fn store_token(&self, installation_id: u64, token: Arc<str>, expires_at: DateTime<Utc>) {
        self.cache
            .lock()
            .expect("cache lock poisoned")
            .insert(installation_id, CachedToken { token, expires_at });
    }

    async fn resolve_installation(
        &self,
        client: &reqwest::Client,
        base_url: &str,
        org: &str,
    ) -> Result<u64, Error> {
        {
            let map = self
                .installations
                .lock()
                .expect("installations lock poisoned");
            if let Some(&id) = map.get(org) {
                return Ok(id);
            }
        }

        let jwt = self.sign_jwt()?;

        let url = format!("{base_url}/orgs/{org}/installation");
        let response = with_common_headers(client.get(&url))
            .bearer_auth(&jwt)
            .header("accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|source| Error::InstallationDiscovery {
                org: org.to_owned(),
                source,
            })?;

        if let Err(source) = response.error_for_status_ref() {
            return Err(Error::InstallationDiscovery {
                org: org.to_owned(),
                source,
            });
        }

        let installation_response: InstallationResponse =
            response
                .json()
                .await
                .map_err(|source| Error::InstallationDiscovery {
                    org: org.to_owned(),
                    source,
                })?;

        self.installations
            .lock()
            .expect("installations lock poisoned")
            .insert(org.to_owned(), installation_response.id);

        return Ok(installation_response.id);

        // Serde structs
        #[derive(Deserialize)]
        struct InstallationResponse {
            id: u64,
        }
    }

    async fn get_token(
        &self,
        client: &reqwest::Client,
        base_url: &str,
        org: &str,
    ) -> Result<Arc<str>, Error> {
        let installation_id = self.resolve_installation(client, base_url, org).await?;

        if let Some(token) = self.cached_token(installation_id) {
            return Ok(token);
        }

        let jwt = self.sign_jwt()?;

        let url = format!("{base_url}/app/installations/{installation_id}/access_tokens");
        let response: TokenResponse = with_common_headers(client.post(&url))
            .bearer_auth(&jwt)
            .header("accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(Error::TokenAcquisition)?
            .error_for_status()
            .map_err(Error::TokenAcquisition)?
            .json()
            .await
            .map_err(Error::TokenAcquisition)?;

        let token: Arc<str> = response.token.into();
        self.store_token(installation_id, Arc::clone(&token), response.expires_at);

        return Ok(token);

        // Serde structs
        #[derive(Deserialize)]
        struct TokenResponse {
            token: String,
            expires_at: DateTime<Utc>,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use jsonwebtoken::{dangerous::insecure_decode, decode_header};
    use rand::{SeedableRng, rngs::ChaCha8Rng};
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey};
    use simple_test_case::test_case;
    use std::sync::LazyLock;

    const APP_ID: u64 = 12345;
    const INSTALLATION_ID: u64 = 67890;
    const TEST_ORG: &str = "test";

    // Generated fresh per test process — never committed to the repo.
    static TEST_PRIVATE_KEY_PEM: LazyLock<String> = LazyLock::new(|| {
        RsaPrivateKey::new(&mut ChaCha8Rng::from_seed([42u8; 32]), 2048)
            .expect("failed to generate RSA test key")
            .to_pkcs1_pem(Default::default())
            .expect("failed to encode RSA test key as PKCS#1 PEM")
            .to_string()
    });

    fn make_auth(token_cache: Option<CachedToken>) -> AppInstallationAuth {
        let mut installations = HashMap::new();
        installations.insert(TEST_ORG.to_owned(), INSTALLATION_ID);

        let mut cache = HashMap::new();
        if let Some(t) = token_cache {
            cache.insert(INSTALLATION_ID, t);
        }

        AppInstallationAuth {
            app_id: APP_ID,
            private_key_pem: TEST_PRIVATE_KEY_PEM.as_str().into(),
            installations: Arc::new(Mutex::new(installations)),
            cache: Arc::new(Mutex::new(cache)),
        }
    }

    #[test]
    fn sign_jwt_success() {
        let app_install_auth = make_auth(None);
        let jwt = app_install_auth.sign_jwt().unwrap();

        let header = decode_header(&jwt).unwrap();
        assert_eq!(header.alg, Algorithm::RS256);

        let data = insecure_decode::<serde_json::Value>(&jwt).unwrap();

        let now = Utc::now().timestamp();
        let iss = data.claims["iss"].as_str().unwrap();
        let iat = data.claims["iat"].as_i64().unwrap();
        let exp = data.claims["exp"].as_i64().unwrap();

        assert_eq!(iss, APP_ID.to_string());
        assert!(iat <= now - 50, "iat should be ~60s in the past, got {iat}");
        assert!(
            exp >= now + 550,
            "exp should be ~600s in the future, got {exp}"
        );
        assert_eq!(exp - iat, 660);
    }

    // A valid cached token is returned without making any HTTP call. We prove this by pointing
    // at a broken base URL — if HTTP were attempted the call would error.
    #[tokio::test]
    async fn get_token_cache_hit_success() {
        let auth = make_auth(Some(CachedToken {
            token: "ghs_cached".into(),
            expires_at: Utc::now() + Duration::hours(1),
        }));

        let token = auth
            .get_token(&reqwest::Client::new(), "http://127.0.0.1:1", TEST_ORG)
            .await
            .unwrap();

        assert_eq!(token.as_ref(), "ghs_cached");
    }

    // An expired cached token causes an HTTP fetch attempt. We prove this by pointing at a
    // broken base URL — the resulting TokenAcquisition error shows the fetch was attempted.
    #[tokio::test]
    async fn get_token_token_cache_miss_attempts_fetch() {
        let auth = make_auth(Some(CachedToken {
            token: "ghs_expired".into(),
            expires_at: Utc::now() - Duration::hours(1),
        }));

        let result = auth
            .get_token(&reqwest::Client::new(), "http://127.0.0.1:1", TEST_ORG)
            .await;

        assert!(
            matches!(result, Err(Error::TokenAcquisition(_))),
            "expected TokenAcquisition error, got {result:?}"
        );
    }

    // An unknown org triggers installation discovery. We prove this by using a broken base URL
    // with no pre-populated installations map — the InstallationDiscovery error shows the
    // discovery request was attempted.
    #[tokio::test]
    async fn get_token_installation_cache_miss_attempts_discovery() {
        let auth = AppInstallationAuth {
            app_id: APP_ID,
            private_key_pem: TEST_PRIVATE_KEY_PEM.as_str().into(),
            installations: Arc::new(Mutex::new(HashMap::new())),
            cache: Arc::new(Mutex::new(HashMap::new())),
        };

        let result = auth
            .get_token(&reqwest::Client::new(), "http://127.0.0.1:1", TEST_ORG)
            .await;

        assert!(
            matches!(result, Err(Error::InstallationDiscovery { .. })),
            "expected InstallationDiscovery error, got {result:?}"
        );
    }

    #[test]
    fn sign_jwt_bad_key_returns_jwt_encoding_error() {
        let auth = AppInstallationAuth {
            app_id: APP_ID,
            private_key_pem: "not a pem key".into(),
            installations: Arc::new(Mutex::new(HashMap::new())),
            cache: Arc::new(Mutex::new(HashMap::new())),
        };
        assert!(
            matches!(auth.sign_jwt(), Err(Error::InvalidRsaKey(_))),
            "expected InvalidRsaKey error for invalid PEM"
        );
    }

    // A token stored via one clone is visible to another clone. Verifies that the Arc<Mutex<...>>
    // is shared across clones rather than each having an independent cache.
    #[tokio::test]
    async fn cloned_client_shares_token_cache() {
        let auth = make_auth(None);
        let clone = auth.clone();

        auth.store_token(
            INSTALLATION_ID,
            "ghs_from_original".into(),
            Utc::now() + Duration::hours(1),
        );

        let token = clone
            .get_token(&reqwest::Client::new(), "http://127.0.0.1:1", TEST_ORG)
            .await
            .unwrap();

        assert_eq!(token.as_ref(), "ghs_from_original");
    }

    // Two orgs with different installation IDs each get their own cached token. Verifies that
    // the token cache is keyed by installation ID, not by org.
    #[tokio::test]
    async fn get_token_different_orgs_keyed_independently() {
        const OTHER_INSTALLATION_ID: u64 = 11111;

        let mut installations = HashMap::new();
        installations.insert(TEST_ORG.to_owned(), INSTALLATION_ID);
        installations.insert("other-org".to_owned(), OTHER_INSTALLATION_ID);

        let mut cache = HashMap::new();
        cache.insert(
            INSTALLATION_ID,
            CachedToken {
                token: "ghs_org_a".into(),
                expires_at: Utc::now() + Duration::hours(1),
            },
        );
        cache.insert(
            OTHER_INSTALLATION_ID,
            CachedToken {
                token: "ghs_org_b".into(),
                expires_at: Utc::now() + Duration::hours(1),
            },
        );

        let auth = AppInstallationAuth {
            app_id: APP_ID,
            private_key_pem: TEST_PRIVATE_KEY_PEM.as_str().into(),
            installations: Arc::new(Mutex::new(installations)),
            cache: Arc::new(Mutex::new(cache)),
        };

        let token_a = auth
            .get_token(&reqwest::Client::new(), "http://127.0.0.1:1", TEST_ORG)
            .await
            .unwrap();
        let token_b = auth
            .get_token(&reqwest::Client::new(), "http://127.0.0.1:1", "other-org")
            .await
            .unwrap();

        assert_eq!(token_a.as_ref(), "ghs_org_a");
        assert_eq!(token_b.as_ref(), "ghs_org_b");
    }

    #[test_case(
        Some("my-branch"),
        "https://api.github.com/repos/org/repo/commits/my-branch";
        "explicit ref"
    )]
    #[test_case(
        None,
        "https://api.github.com/repos/org/repo/commits/HEAD";
        "default branch"
    )]
    #[test]
    fn commit_url_resolves_the_ref(git_ref: Option<&str>, expected: &str) {
        assert_eq!(commit_url(GITHUB_API_URL, "org", "repo", git_ref), expected);
    }
}
