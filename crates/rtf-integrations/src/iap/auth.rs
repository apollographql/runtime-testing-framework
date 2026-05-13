//! User-OAuth loopback flow that mints ID tokens for the IAP OAuth client.
//!
//! The same Web OAuth client that IAP is configured with also drives the CLI's
//! consent flow. Google's default behaviour is to issue ID tokens with `aud`
//! equal to the requesting `client_id`, so the resulting tokens are already
//! aimed at the IAP audience — no `audience` extra param needed.

use crate::iap::client::{Error, Result};
use chrono::{DateTime, Duration, Utc};
use oauth2::basic::{
    BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
    BasicTokenType,
};
use oauth2::{
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, ExtraTokenFields, PkceCodeChallenge, RedirectUrl, RefreshToken, Scope,
    StandardRevocableToken, StandardTokenResponse, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{env, fs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::form_urlencoded;

const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const TOKEN_CACHE_SUBDIR: &str = "rtf";
const TOKEN_CACHE_FILENAME: &str = "iap_credentials.json";
const LOOPBACK_TIMEOUT_SECS: u64 = 300;
const ID_TOKEN_EXPIRY_SKEW_SECS: i64 = 60;

/// Inputs for the user-OAuth flow: the IAP OAuth client credentials that both
/// drive user consent and determine the audience of the resulting ID token.
pub(super) struct OauthConfig<'a> {
    pub client_id: &'a str,
    pub client_secret: &'a str,
}

/// Persisted token state at `~/.config/rtf/iap_credentials.json`.
#[derive(Debug, Serialize, Deserialize)]
struct CachedTokens {
    client_id: String,
    id_token: String,
    id_token_expires_at: DateTime<Utc>,
    refresh_token: String,
}

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
type GoogleOauthClient = Client<
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
pub(super) async fn id_token(config: &OauthConfig<'_>) -> Result<String> {
    if adc_is_service_account_like() {
        return service_account_id_token(config.client_id).await;
    }

    if let Some(cached) = load_cache()
        && cached.client_id == config.client_id
    {
        if cached.id_token_expires_at > Utc::now() + Duration::seconds(ID_TOKEN_EXPIRY_SKEW_SECS) {
            return Ok(cached.id_token);
        }
        if let Ok(refreshed) = refresh_id_token(&cached.refresh_token, config).await {
            save_cache(&refreshed)?;
            return Ok(refreshed.id_token);
        }
    }
    let tokens = run_oauth_flow(config).await?;
    save_cache(&tokens)?;

    Ok(tokens.id_token)
}

/// `true` when ADC describes a credential type whose ID tokens IAP will accept
/// without an interactive consent flow.
///
/// Reads the ADC file at `$GOOGLE_APPLICATION_CREDENTIALS` (if set) or the
/// platform-default location, parses just the `type` field, and matches
/// against the two service-account variants. Anything else — missing file,
/// parse failure, `authorized_user`, `external_account` — returns `false` so
/// the caller falls back to the user OAuth flow.
fn adc_is_service_account_like() -> bool {
    let Some(path) = adc_path() else {
        return false;
    };
    let Ok(bytes) = fs::read(&path) else {
        return false;
    };

    #[derive(Deserialize)]
    struct AdcType<'a> {
        #[serde(rename = "type", borrow)]
        kind: Option<&'a str>,
    }

    let Ok(parsed) = serde_json::from_slice::<AdcType<'_>>(&bytes) else {
        return false;
    };

    matches!(
        parsed.kind,
        Some("service_account") | Some("impersonated_service_account")
    )
}

fn adc_path() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("GOOGLE_APPLICATION_CREDENTIALS")
        && !explicit.is_empty()
    {
        return Some(PathBuf::from(explicit));
    }
    let home = env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("gcloud")
            .join("application_default_credentials.json"),
    )
}

/// Mint an ID token aimed at `audience` using the service account that ADC
/// resolves to. No browser, no consent screen, no on-disk cache (the SDK
/// caches tokens in-memory and they're short-lived enough that re-minting
/// per invocation is cheap).
async fn service_account_id_token(audience: &str) -> Result<String> {
    let credentials = google_cloud_auth::credentials::idtoken::Builder::new(audience.to_owned())
        .build()
        .map_err(|e| {
            Error::Adc(format!(
                "could not build service-account ID token credentials: {e}"
            ))
        })?;
    credentials
        .id_token()
        .await
        .map_err(|e| Error::Adc(format!("could not mint service-account ID token: {e}")))
}

/// Build the `oauth2::Client` for Google's endpoints. `redirect_uri` is only
/// needed for the authorization-code grant; refresh-token grants pass `None`.
fn build_oauth_client(
    config: &OauthConfig<'_>,
    redirect_uri: Option<&str>,
) -> Result<GoogleOauthClient> {
    let mut client = Client::new(ClientId::new(config.client_id.to_owned()))
        .set_client_secret(ClientSecret::new(config.client_secret.to_owned()))
        .set_auth_uri(
            AuthUrl::new(GOOGLE_AUTH_ENDPOINT.to_owned())
                .map_err(|e| Error::OauthFlow(format!("invalid auth URL: {e}")))?,
        )
        .set_token_uri(
            TokenUrl::new(GOOGLE_TOKEN_ENDPOINT.to_owned())
                .map_err(|e| Error::OauthFlow(format!("invalid token URL: {e}")))?,
        );

    if let Some(uri) = redirect_uri {
        client = client.set_redirect_uri(
            RedirectUrl::new(uri.to_owned())
                .map_err(|e| Error::OauthFlow(format!("invalid redirect URI: {e}")))?,
        );
    }

    Ok(client)
}

/// Build the HTTP client used for token-endpoint calls. Redirects are disabled
/// to avoid SSRF (per the `oauth2` crate's recommendation).
///
/// This uses `oauth2`'s bundled reqwest 0.12 (not the workspace's 0.13) because
/// that's what `AsyncHttpClient` is implemented for. The duplication is
/// confined to this OAuth flow.
fn build_oauth_http_client() -> Result<oauth2::reqwest::Client> {
    oauth2::reqwest::ClientBuilder::new()
        .redirect(oauth2::reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| Error::OauthFlow(format!("could not build OAuth HTTP client: {e}")))
}

async fn run_oauth_flow(config: &OauthConfig<'_>) -> Result<CachedTokens> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::OauthFlow(format!("could not bind loopback listener: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::OauthFlow(format!("could not read loopback port: {e}")))?
        .port();
    // `localhost` (not `127.0.0.1`) so that Google's special-case rule for
    // Web OAuth clients accepts the redirect — the Web client only needs
    // `http://localhost` (any port) registered as an authorized redirect URI.
    let redirect_uri = format!("http://localhost:{port}");

    let oauth_client = build_oauth_client(config, Some(&redirect_uri))?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, csrf_token) = oauth_client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("openid".to_owned()))
        .add_scope(Scope::new("email".to_owned()))
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .set_pkce_challenge(pkce_challenge)
        .url();

    eprintln!("Opening browser to authenticate with IAP…");
    eprintln!("If the browser does not open, visit this URL:\n  {auth_url}\n");
    let _ = open::that(auth_url.as_str());

    let (code, returned_state) = tokio::time::timeout(
        std::time::Duration::from_secs(LOOPBACK_TIMEOUT_SECS),
        accept_oauth_callback(listener),
    )
    .await
    .map_err(|_| {
        Error::OauthFlow(format!(
            "OAuth flow timed out after {LOOPBACK_TIMEOUT_SECS}s without a redirect"
        ))
    })??;

    if returned_state != csrf_token.secret().as_str() {
        return Err(Error::OauthFlow(
            "OAuth state mismatch — aborting to prevent CSRF".to_owned(),
        ));
    }

    let http_client = build_oauth_http_client()?;
    let token_result = oauth_client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| Error::OauthFlow(format!("Google token endpoint: {e}")))?;

    let refresh_token = token_result
        .refresh_token()
        .ok_or_else(|| {
            Error::OauthFlow(
                "Google did not return a refresh_token; check that the OAuth consent screen \
                 grants offline access"
                    .to_owned(),
            )
        })?
        .secret()
        .to_owned();

    Ok(CachedTokens {
        client_id: config.client_id.to_owned(),
        id_token: token_result.extra_fields().id_token.clone(),
        id_token_expires_at: expires_at(&token_result),
        refresh_token,
    })
}

async fn refresh_id_token(refresh_token: &str, config: &OauthConfig<'_>) -> Result<CachedTokens> {
    let oauth_client = build_oauth_client(config, None)?;
    let http_client = build_oauth_http_client()?;

    let token_result = oauth_client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_owned()))
        .request_async(&http_client)
        .await
        .map_err(|e| Error::OauthFlow(format!("Google token endpoint: {e}")))?;

    // Refresh-token grants may or may not return a new refresh_token; keep the
    // existing one if Google didn't rotate it.
    let next_refresh_token = token_result
        .refresh_token()
        .map(|t| t.secret().to_owned())
        .unwrap_or_else(|| refresh_token.to_owned());

    Ok(CachedTokens {
        client_id: config.client_id.to_owned(),
        id_token: token_result.extra_fields().id_token.clone(),
        id_token_expires_at: expires_at(&token_result),
        refresh_token: next_refresh_token,
    })
}

/// Convert the SDK's `expires_in` into an absolute UTC timestamp, defaulting to
/// one hour out when Google omits the field (per RFC 6749 §5.1 it's only
/// RECOMMENDED, not REQUIRED).
fn expires_at(response: &GoogleTokenResponse) -> DateTime<Utc> {
    let expires_in = response
        .expires_in()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(3600);
    Utc::now() + Duration::seconds(expires_in)
}

async fn accept_oauth_callback(listener: TcpListener) -> Result<(String, String)> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| Error::OauthFlow(format!("loopback accept failed: {e}")))?;

    let mut buf = vec![0u8; 16 * 1024];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| Error::OauthFlow(format!("loopback read failed: {e}")))?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| Error::OauthFlow("empty loopback request".to_owned()))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| Error::OauthFlow("missing path in loopback request".to_owned()))?;
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
        return Err(Error::OauthFlow(format!("authorization denied: {err}")));
    }
    let code = code.ok_or_else(|| Error::OauthFlow("missing code in OAuth callback".to_owned()))?;
    let state =
        state.ok_or_else(|| Error::OauthFlow("missing state in OAuth callback".to_owned()))?;

    Ok((code, state))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── Disk cache (~/.config/rtf/iap_credentials.json, mode 0600) ─────────────

fn cache_dir() -> Option<PathBuf> {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join(TOKEN_CACHE_SUBDIR));
    }
    let home = env::var("HOME").ok().filter(|h| !h.is_empty())?;

    Some(PathBuf::from(home).join(".config").join(TOKEN_CACHE_SUBDIR))
}

fn cache_path() -> Option<PathBuf> {
    cache_dir().map(|d| d.join(TOKEN_CACHE_FILENAME))
}

fn load_cache() -> Option<CachedTokens> {
    let path = cache_path()?;
    let bytes = fs::read(&path).ok()?;

    serde_json::from_slice(&bytes).ok()
}

fn save_cache(tokens: &CachedTokens) -> Result<()> {
    let path = cache_path().ok_or_else(|| {
        Error::OauthFlow(
            "cannot determine token cache location (HOME and XDG_CONFIG_HOME both unset)"
                .to_owned(),
        )
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(Error::TokenCache)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }

    let json = serde_json::to_vec_pretty(tokens)
        .map_err(|e| Error::TokenCache(std::io::Error::other(e)))?;
    fs::write(&path, &json).map_err(Error::TokenCache)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(Error::TokenCache)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_query_value(value: &str) -> String {
        let query = format!("v={value}");
        form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    }

    #[test]
    fn query_decoding_handles_plus_and_hex() {
        // Documents how the OAuth callback parser decodes redirect query values.
        assert_eq!(decode_query_value("hello+world"), "hello world");
        assert_eq!(decode_query_value("a%20b"), "a b");
        assert_eq!(decode_query_value("%2F%3A"), "/:");
        assert_eq!(decode_query_value("noop"), "noop");
    }

    #[test]
    fn query_decoding_passes_through_malformed_percent() {
        assert_eq!(decode_query_value("%ZZ"), "%ZZ");
    }

    #[test]
    fn html_escape_escapes_dangerous_chars() {
        assert_eq!(
            html_escape("<script>alert(\"x\")</script>"),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn adc_classification_via_env_var() {
        // Temporarily point GOOGLE_APPLICATION_CREDENTIALS at a tempfile whose
        // contents we control, exercise the parser, restore the env.
        use std::io::Write;
        let dir = std::env::temp_dir();

        fn run_case(dir: &std::path::Path, name: &str, contents: &str) -> bool {
            let path = dir.join(name);
            std::fs::File::create(&path)
                .unwrap()
                .write_all(contents.as_bytes())
                .unwrap();
            // SAFETY: tests in this crate touching this env var are all
            // gated through this helper and run sequentially within the test.
            unsafe { std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", &path) };
            let result = adc_is_service_account_like();
            // SAFETY: see above — same single-threaded scope.
            unsafe { std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS") };
            std::fs::remove_file(&path).ok();
            result
        }

        assert!(run_case(
            &dir,
            "rtf_adc_sa.json",
            r#"{"type":"service_account","client_email":"x@y.iam.gserviceaccount.com"}"#
        ));
        assert!(run_case(
            &dir,
            "rtf_adc_impersonated.json",
            r#"{"type":"impersonated_service_account"}"#
        ));
        assert!(!run_case(
            &dir,
            "rtf_adc_user.json",
            r#"{"type":"authorized_user"}"#
        ));
        assert!(!run_case(
            &dir,
            "rtf_adc_external.json",
            r#"{"type":"external_account"}"#
        ));
        assert!(!run_case(&dir, "rtf_adc_no_type.json", r#"{}"#));
        assert!(!run_case(&dir, "rtf_adc_garbage.json", r#"not json"#));
    }

    #[test]
    fn authorize_url_carries_expected_oauth_params() {
        let config = OauthConfig {
            client_id: "CID",
            client_secret: "SECRET",
        };
        let client = build_oauth_client(&config, Some("http://localhost:1234")).unwrap();
        let (pkce, _) = PkceCodeChallenge::new_random_sha256();
        let (url, _csrf) = client
            .authorize_url(|| CsrfToken::new("ST".to_owned()))
            .add_scope(Scope::new("openid".to_owned()))
            .add_scope(Scope::new("email".to_owned()))
            .add_extra_param("access_type", "offline")
            .add_extra_param("prompt", "consent")
            .set_pkce_challenge(pkce)
            .url();
        let url = url.as_str();
        assert!(url.starts_with(GOOGLE_AUTH_ENDPOINT), "got {url}");
        assert!(url.contains("client_id=CID"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1234"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=ST"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        // openid + email come out URL-encoded; the order is preserved.
        assert!(url.contains("scope=openid+email"));
    }
}
