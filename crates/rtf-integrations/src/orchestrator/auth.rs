use crate::orchestrator::{AdcError, Error, OauthError, Result};
use chrono::{DateTime, Utc};
use google_cloud_auth::credentials::{external_account, idtoken};
use oauth2::{
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, ExtraTokenFields, PkceCodeChallenge, RedirectUrl, RefreshToken, Scope,
    StandardRevocableToken, StandardTokenResponse, TokenResponse, TokenUrl,
    basic::{
        BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
        BasicTokenType,
    },
    reqwest::{ClientBuilder, redirect::Policy},
    url::form_urlencoded,
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, Permissions},
    io,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use tracing::debug;

const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const TOKEN_CACHE_SUBDIR: &str = "rtf";
const TOKEN_CACHE_FILENAME: &str = "orchestrator_credentials.json";
const LOOPBACK_TIMEOUT_SECS: u64 = 300;
const ID_TOKEN_EXPIRY_SKEW_SECS: u64 = 60;

/// Extra fields beyond RFC 6749 that Google returns from the token endpoint.
///
/// `id_token` is the OIDC ID token; everything else in the response is covered
/// by [`StandardTokenResponse`].
#[derive(Clone, Debug, Deserialize, Serialize)]
struct GoogleExtraFields {
    id_token: String,
}

impl ExtraTokenFields for GoogleExtraFields {}

type GoogleTokenResponse = StandardTokenResponse<GoogleExtraFields, BasicTokenType>;

/// `oauth2` client configured with Google's endpoints and our custom
/// `id_token`-aware response type. The two `EndpointSet` markers reflect the
/// auth and token URLs being populated by [`build_oauth_client`].
type GoogleOauthClient = oauth2::Client<
    BasicErrorResponse,
    GoogleTokenResponse,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;

/// Application Default Credentials type, loaded once at client initialisation.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AdcCredentials {
    /// A service account key file. `idtoken::Builder` mints ID tokens directly
    /// from the key material.
    ServiceAccount,
    /// An impersonated service account. `idtoken::Builder` handles this flow
    /// via the IAM Credentials API.
    ImpersonatedServiceAccount,
    /// Workload Identity Federation (external_account) credentials, used in CI
    /// via google-github-actions/auth. ID tokens are not supported by the SDK
    /// for this type; instead we get an access token and call IAM signJwt.
    ExternalAccount,
    /// A user credential from `gcloud auth application-default login`. Requires
    /// the interactive OAuth loopback flow.
    AuthorizedUser,
}

impl AdcCredentials {
    /// Locate, read, and parse the ADC file.
    pub(super) fn load() -> Result<Self> {
        let path = Self::path()?;

        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(AdcError::NotAuthenticated.into());
            }
            Err(e) => return Err(AdcError::ReadFailed(e).into()),
        };

        serde_json::from_slice(&bytes)
            .map_err(AdcError::ParseFailed)
            .map_err(Into::into)
    }

    pub(super) fn is_service_account_like(&self) -> bool {
        matches!(
            self,
            Self::ServiceAccount | Self::ImpersonatedServiceAccount
        )
    }

    fn path() -> Result<PathBuf> {
        if let Ok(explicit) = env::var("GOOGLE_APPLICATION_CREDENTIALS")
            && !explicit.is_empty()
        {
            return Ok(PathBuf::from(explicit));
        }

        let home = env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .ok_or(AdcError::NoHomeDir)?;

        Ok(PathBuf::from(home)
            .join(".config")
            .join("gcloud")
            .join("application_default_credentials.json"))
    }
}

/// Persisted token state at `~/.config/rtf/iap_credentials.json`.
#[derive(Debug, Serialize, Deserialize)]
struct CachedTokens {
    client_id: String,
    id_token: String,
    id_token_expires_at: DateTime<Utc>,
    refresh_token: String,
}

impl CachedTokens {
    fn cache_path() -> Result<PathBuf> {
        Ok(xdg::BaseDirectories::with_prefix(TOKEN_CACHE_SUBDIR)
            .place_config_file(TOKEN_CACHE_FILENAME)
            .map_err(|_| OauthError::NoCacheLocation)?)
    }

    fn load() -> Result<Option<Self>> {
        let path = Self::cache_path()?;

        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::TokenCache(e)),
        };

        let tokens =
            serde_json::from_slice(&bytes).map_err(|e| Error::TokenCache(io::Error::other(e)))?;

        Ok(Some(tokens))
    }

    async fn refresh(&self, client_secret: &str) -> Result<Self> {
        let oauth_client = build_oauth_client(&self.client_id, client_secret)?;
        let http_client = build_oauth_http_client()?;

        let token_result = oauth_client
            .exchange_refresh_token(&RefreshToken::new(self.refresh_token.clone()))
            .request_async(&http_client)
            .await
            .map_err(|e| OauthError::TokenEndpoint(e.to_string()))?;

        // Refresh-token grants may or may not return a new refresh_token; keep the
        // existing one if Google didn't rotate it.
        let next_refresh_token = token_result
            .refresh_token()
            .map(|t| t.secret().to_owned())
            .unwrap_or_else(|| self.refresh_token.clone());

        Ok(Self {
            client_id: self.client_id.clone(),
            id_token: token_result.extra_fields().id_token.clone(),
            id_token_expires_at: expires_at(&token_result),
            refresh_token: next_refresh_token,
        })
    }

    fn save(&self) -> Result<()> {
        let path = Self::cache_path()?;

        if let Some(parent) = path.parent() {
            let _ = fs::set_permissions(parent, Permissions::from_mode(0o700));
        }

        let json =
            serde_json::to_vec_pretty(self).map_err(|e| Error::TokenCache(io::Error::other(e)))?;
        fs::write(&path, &json).map_err(Error::TokenCache)?;

        fs::set_permissions(&path, Permissions::from_mode(0o600)).map_err(Error::TokenCache)?;

        Ok(())
    }
}

/// Obtain an IAP-audience OIDC ID token for the current principal.
///
/// When Application Default Credentials describe a service account (directly or
/// via impersonation), mints an ID token through Google's service-account
/// token endpoint and skips user consent — this is the non-interactive path
/// used by CI.
///
/// Otherwise (a human user signed in via `gcloud auth application-default
/// login`), tries the on-disk cache, then a refresh-token grant, then runs the
/// full loopback OAuth flow (which opens a browser).
pub(super) async fn id_token(
    adc: &AdcCredentials,
    client_id: &str,
    client_secret: &str,
) -> Result<String> {
    if matches!(adc, AdcCredentials::ExternalAccount) {
        return external_account_iap_token(client_id).await;
    }

    if adc.is_service_account_like() {
        return service_account_id_token(client_id).await;
    }

    if let Some(cache) = CachedTokens::load()? {
        if cache.id_token_expires_at > Utc::now() + Duration::from_secs(ID_TOKEN_EXPIRY_SKEW_SECS) {
            return Ok(cache.id_token);
        }

        if let Ok(refreshed) = cache.refresh(client_secret).await {
            refreshed.save()?;

            return Ok(refreshed.id_token);
        }
    }

    let tokens = run_oauth_flow(client_id, client_secret).await?;
    tokens.save()?;

    Ok(tokens.id_token)
}

/// Mint an ID token aimed at `audience` using the service account that ADC
/// resolves to. No browser, no consent screen, no on-disk cache (the SDK
/// caches tokens in-memory and they're short-lived enough that re-minting
/// per invocation is cheap).
async fn service_account_id_token(audience: &str) -> Result<String> {
    let credentials = idtoken::Builder::new(audience.to_owned())
        .build()
        .map_err(|e| AdcError::CredentialsBuild(e.to_string()))?;

    Ok(credentials
        .id_token()
        .await
        .map_err(|e| AdcError::TokenMint(e.to_string()))?)
}

/// Obtain an IAP Bearer token for WIF (external_account) credentials.
///
/// The google-cloud-auth SDK does not support ID tokens from external_account
/// credentials. Instead: get an access token from the WIF credential, then
/// call the IAM Credentials API `signJwt` method to produce a self-signed JWT
/// that IAP accepts as a Bearer token.
async fn external_account_iap_token(iap_client_id: &str) -> Result<String> {
    let path = AdcCredentials::path()?;
    let bytes = fs::read(&path).map_err(AdcError::ReadFailed)?;

    // Parse out the SA email from service_account_impersonation_url.
    // URL format: https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/EMAIL:generateAccessToken
    #[derive(Deserialize)]
    struct ImpersonationFields {
        service_account_impersonation_url: Option<String>,
    }
    let fields: ImpersonationFields =
        serde_json::from_slice(&bytes).map_err(AdcError::ParseFailed)?;
    let impersonation_url = fields
        .service_account_impersonation_url
        .ok_or(AdcError::NoImpersonationUrl)?;
    let sa_email = extract_sa_email(&impersonation_url)?;

    // Get an access token from the WIF credentials.
    let config: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(AdcError::ParseFailed)?;
    let creds = external_account::Builder::new(config)
        .build_access_token_credentials()
        .inspect_err(|e| debug!(error = ?e, "WIF: failed to build access token credentials"))
        .map_err(|e| AdcError::CredentialsBuild(e.to_string()))?;
    let access_token = creds
        .access_token()
        .await
        .inspect_err(|e| debug!(error = ?e, "WIF: access_token() failed"))
        .map_err(|e| AdcError::TokenMint(e.to_string()))?
        .token;

    // Generate an OIDC ID token via IAM Credentials API. IAP requires a
    // Google-issued OIDC token; self-signed JWTs (signJwt) are only accepted
    // by a subset of Google services.
    let url = format!(
        "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{}:generateIdToken",
        sa_email
    );
    let body = serde_json::json!({
        "audience": iap_client_id,
        "includeEmail": true,
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&access_token)
        .json(&body)
        .send()
        .await
        .inspect_err(|e| debug!(error = ?e, "WIF: generateIdToken HTTP request failed"))
        .map_err(|e| AdcError::TokenMint(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(
            AdcError::TokenMint(format!("generateIdToken failed ({status}): {text}")).into(),
        );
    }

    #[derive(Deserialize)]
    struct GenerateIdTokenResponse {
        token: String,
    }
    let token_resp: GenerateIdTokenResponse = resp
        .json()
        .await
        .map_err(|e| AdcError::TokenMint(e.to_string()))?;

    Ok(token_resp.token)
}

/// Extract the service account email from a `generateAccessToken` impersonation URL.
fn extract_sa_email(url: &str) -> Result<String> {
    url.split("/serviceAccounts/")
        .nth(1)
        .and_then(|s| s.split(':').next())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
        .ok_or_else(|| AdcError::InvalidImpersonationUrl(url.to_owned()).into())
}

/// Build the `oauth2::Client` for Google's endpoints. `redirect_uri` is only
/// needed for the authorization-code grant; refresh-token grants pass `None`.
fn build_oauth_client(client_id: &str, client_secret: &str) -> Result<GoogleOauthClient> {
    let client = Client::new(ClientId::new(client_id.to_string()))
        .set_client_secret(ClientSecret::new(client_secret.to_string()))
        .set_auth_uri(AuthUrl::new(GOOGLE_AUTH_ENDPOINT.to_string()).map_err(|e| {
            OauthError::InvalidUrl {
                endpoint: "auth",
                message: e.to_string(),
            }
        })?)
        .set_token_uri(
            TokenUrl::new(GOOGLE_TOKEN_ENDPOINT.to_owned()).map_err(|e| {
                OauthError::InvalidUrl {
                    endpoint: "token",
                    message: e.to_string(),
                }
            })?,
        );

    Ok(client)
}

/// Build the HTTP client used for token-endpoint calls. Redirects are disabled
/// to avoid SSRF (per the `oauth2` crate's recommendation).
///
/// This uses `oauth2`'s bundled reqwest 0.12 (not the workspace's 0.13) because
/// that's what `AsyncHttpClient` is implemented for. The duplication is
/// confined to this OAuth flow.
fn build_oauth_http_client() -> Result<oauth2::reqwest::Client> {
    ClientBuilder::new()
        .redirect(Policy::none())
        .build()
        .map_err(|e| OauthError::HttpClientBuild(e.to_string()))
        .map_err(Into::into)
}

/// Run the user OAuth flow to get IAP token
async fn run_oauth_flow(client_id: &str, client_secret: &str) -> Result<CachedTokens> {
    let oauth_client = build_oauth_client(client_id, client_secret)?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(OauthError::LoopbackBind)?;
    let port = listener
        .local_addr()
        .map_err(OauthError::LoopbackPort)?
        .port();

    // `localhost` (not `127.0.0.1`) so that Google's special-case rule for
    // Web OAuth clients accepts the redirect — the Web client only needs
    // `http://localhost` (any port) registered as an authorized redirect URI.
    let redirect_uri = format!("http://localhost:{port}");
    let oauth_client = oauth_client.set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_string())
            .map_err(|e| OauthError::InvalidRedirectUri(e.to_string()))?,
    );

    let (auth_url, csrf_token) = oauth_client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("openid".to_owned()))
        .add_scope(Scope::new("email".to_owned()))
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .set_pkce_challenge(pkce_challenge)
        .url();

    println!("Opening browser to authenticate with IAP…");
    println!("If the browser does not open, visit this URL:\n  {auth_url}\n");
    let _ = open::that(auth_url.as_str());

    let (code, returned_state) = timeout(
        Duration::from_secs(LOOPBACK_TIMEOUT_SECS),
        accept_oauth_callback(listener),
    )
    .await
    .map_err(|_| OauthError::FlowTimeout(LOOPBACK_TIMEOUT_SECS))??;

    if returned_state != csrf_token.secret().as_str() {
        return Err(OauthError::CsrfMismatch.into());
    }

    let http_client = build_oauth_http_client()?;

    let token_result = oauth_client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| OauthError::TokenEndpoint(e.to_string()))?;

    let refresh_token = token_result
        .refresh_token()
        .ok_or(OauthError::NoRefreshToken)?
        .secret()
        .to_owned();

    Ok(CachedTokens {
        client_id: client_id.to_string(),
        id_token: token_result.extra_fields().id_token.clone(),
        id_token_expires_at: expires_at(&token_result),
        refresh_token,
    })
}

async fn accept_oauth_callback(listener: TcpListener) -> Result<(String, String)> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(OauthError::LoopbackAccept)?;

    let mut buf = vec![0u8; 16 * 1024];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(OauthError::LoopbackRead)?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let (code, state, oauth_error) = parse_loopback_request(&request)?;

    let body = match oauth_error.as_ref() {
        Some(err) => format!(
            "<!doctype html><html><body><h1>Authentication failed</h1><p>{}</p></body></html>",
            html_escape(err)
        ),
        None => "<!doctype html><html><body><h1>Authentication complete</h1>\
                 <p>You can close this tab.</p>\
                 <script>window.close();</script></body></html>"
            .to_owned(),
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;

    if let Some(err) = oauth_error {
        return Err(OauthError::AuthorizationDenied(err).into());
    }
    let code = code.ok_or(OauthError::MissingCode)?;
    let state = state.ok_or(OauthError::MissingState)?;

    Ok((code, state))
}

/// Parse the path and query parameters from a raw loopback HTTP request line.
///
/// Returns `(code, state, oauth_error)`, any of which may be `None` if the
/// corresponding query parameter was absent.
fn parse_loopback_request(
    request: &str,
) -> Result<(Option<String>, Option<String>, Option<String>)> {
    let first_line = request
        .lines()
        .next()
        .ok_or(OauthError::MalformedCallback("empty loopback request"))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or(OauthError::MalformedCallback(
            "missing path in loopback request",
        ))?;
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");

    let mut code = None;
    let mut state = None;
    let mut oauth_error = None;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => oauth_error = Some(value.into_owned()),
            _ => {}
        }
    }

    Ok((code, state, oauth_error))
}

/// Convert the SDK's `expires_in` into an absolute UTC timestamp, defaulting to
/// one hour out when Google omits the field (per RFC 6749 §5.1 it's only
/// RECOMMENDED, not REQUIRED).
fn expires_at(response: &GoogleTokenResponse) -> DateTime<Utc> {
    let expires_in = response.expires_in().map(|d| d.as_secs()).unwrap_or(3600);

    Utc::now() + Duration::from_secs(expires_in)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_loopback_request_code_and_state() {
        let req = "GET /?code=auth_code_123&state=csrf_token HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (code, state, error) = parse_loopback_request(req).unwrap();

        assert_eq!(code.as_deref(), Some("auth_code_123"));
        assert_eq!(state.as_deref(), Some("csrf_token"));
        assert!(error.is_none());
    }

    #[test]
    fn parse_loopback_request_error_param() {
        let req = "GET /?error=access_denied&state=csrf_token HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (code, state, error) = parse_loopback_request(req).unwrap();

        assert!(code.is_none());
        assert_eq!(state.as_deref(), Some("csrf_token"));
        assert_eq!(error.as_deref(), Some("access_denied"));
    }

    #[test]
    fn parse_loopback_request_no_query_string() {
        let req = "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (code, state, error) = parse_loopback_request(req).unwrap();

        assert!(code.is_none());
        assert!(state.is_none());
        assert!(error.is_none());
    }

    #[test]
    fn parse_loopback_request_url_encoded_values() {
        let req = "GET /?code=a%2Fb%2Bc&state=x%3Dy HTTP/1.1\r\n\r\n";
        let (code, state, _) = parse_loopback_request(req).unwrap();

        assert_eq!(code.as_deref(), Some("a/b+c"));
        assert_eq!(state.as_deref(), Some("x=y"));
    }

    #[test]
    fn parse_loopback_request_empty_request_is_malformed() {
        let err = parse_loopback_request("").unwrap_err();

        assert!(
            err.to_string().contains("empty loopback request"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_loopback_request_missing_path_is_malformed() {
        // Only one token on the request line — no path element
        let req = "GET\r\n\r\n";
        let err = parse_loopback_request(req).unwrap_err();

        assert!(
            err.to_string().contains("missing path in loopback request"),
            "unexpected error: {err}"
        );
    }
}
