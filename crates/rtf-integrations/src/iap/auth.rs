use crate::iap::{Error, Result};
use chrono::{DateTime, Duration, Utc};
use google_cloud_auth::credentials::idtoken;
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
use std::{env, fs, io, path::PathBuf, time};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const TOKEN_CACHE_SUBDIR: &str = "rtf";
const TOKEN_CACHE_FILENAME: &str = "iap_credentials.json";
const LOOPBACK_TIMEOUT_SECS: u64 = 300;
const ID_TOKEN_EXPIRY_SKEW_SECS: i64 = 60;

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

/// Persisted token state at `~/.config/rtf/iap_credentials.json`.
#[derive(Debug, Serialize, Deserialize)]
struct CachedTokens {
    client_id: String,
    id_token: String,
    id_token_expires_at: DateTime<Utc>,
    refresh_token: String,
}

impl CachedTokens {
    fn cache_path() -> Option<PathBuf> {
        let dir = if let Ok(xdg) = env::var("XDG_CONFIG_HOME")
            && !xdg.is_empty()
        {
            PathBuf::from(xdg).join(TOKEN_CACHE_SUBDIR)
        } else {
            let home = env::var("HOME").ok().filter(|h| !h.is_empty())?;
            PathBuf::from(home).join(".config").join(TOKEN_CACHE_SUBDIR)
        };

        Some(dir.join(TOKEN_CACHE_FILENAME))
    }

    fn load() -> Result<Option<Self>> {
        let Some(path) = Self::cache_path() else {
            return Ok(None);
        };

        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::TokenCache(e)),
        };

        let tokens =
            serde_json::from_slice(&bytes).map_err(|e| Error::TokenCache(io::Error::other(e)))?;

        Ok(Some(tokens))
    }

    fn save(&self) -> Result<()> {
        let path = Self::cache_path().ok_or_else(|| {
            Error::OauthFlow(
                "cannot determine token cache location (HOME and XDG_CONFIG_HOME both unset)"
                    .to_owned(),
            )
        })?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(Error::TokenCache)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| Error::TokenCache(std::io::Error::other(e)))?;
        fs::write(&path, &json).map_err(Error::TokenCache)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(Error::TokenCache)?;
        }

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
pub(super) async fn id_token(client_id: &str, client_secret: &str) -> Result<String> {
    if adc_is_service_account_like()? {
        return service_account_id_token(client_id).await;
    }

    if let Some(cache) = CachedTokens::load()? {
        if cache.id_token_expires_at > Utc::now() + Duration::seconds(ID_TOKEN_EXPIRY_SKEW_SECS) {
            return Ok(cache.id_token);
        }

        if let Ok(refreshed) =
            refresh_id_token(client_id, client_secret, &cache.refresh_token).await
        {
            refreshed.save()?;

            return Ok(refreshed.id_token);
        }
    }

    let tokens = run_oauth_flow(client_id, client_secret).await?;
    tokens.save()?;

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
fn adc_is_service_account_like() -> Result<bool> {
    let adc_path = if let Ok(explicit) = env::var("GOOGLE_APPLICATION_CREDENTIALS")
        && !explicit.is_empty()
    {
        PathBuf::from(explicit)
    } else {
        let home = env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .ok_or(Error::Adc("unable to find home dir".to_string()))?;

        PathBuf::from(home)
            .join(".config")
            .join("gcloud")
            .join("application_default_credentials.json")
    };

    let bytes = fs::read(adc_path)?;
    let parsed = serde_json::from_slice::<AdcType<'_>>(&bytes)?;

    let res = matches!(
        parsed.kind,
        Some("service_account") | Some("impersonated_service_account")
    );

    return Ok(res);

    #[derive(Deserialize)]
    struct AdcType<'a> {
        #[serde(rename = "type", borrow)]
        kind: Option<&'a str>,
    }
}

/// Mint an ID token aimed at `audience` using the service account that ADC
/// resolves to. No browser, no consent screen, no on-disk cache (the SDK
/// caches tokens in-memory and they're short-lived enough that re-minting
/// per invocation is cheap).
async fn service_account_id_token(audience: &str) -> Result<String> {
    let credentials = idtoken::Builder::new(audience.to_owned())
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
fn build_oauth_client(client_id: &str, client_secret: &str) -> Result<GoogleOauthClient> {
    let client = Client::new(ClientId::new(client_id.to_string()))
        .set_client_secret(ClientSecret::new(client_secret.to_string()))
        .set_auth_uri(
            AuthUrl::new(GOOGLE_AUTH_ENDPOINT.to_string())
                .map_err(|e| Error::OauthFlow(format!("invalid auth URL: {e}")))?,
        )
        .set_token_uri(
            TokenUrl::new(GOOGLE_TOKEN_ENDPOINT.to_owned())
                .map_err(|e| Error::OauthFlow(format!("invalid token URL: {e}")))?,
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
        .map_err(|e| Error::OauthFlow(format!("could not build OAuth HTTP client: {e}")))
}

/// Run the user OAuth flow to get IAP token
async fn run_oauth_flow(client_id: &str, client_secret: &str) -> Result<CachedTokens> {
    let oauth_client = build_oauth_client(client_id, client_secret)?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

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
    let oauth_client = oauth_client.set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_string())
            .map_err(|e| Error::OauthFlow(format!("invalid redirect URI: {e}")))?,
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
        time::Duration::from_secs(LOOPBACK_TIMEOUT_SECS),
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

async fn refresh_id_token(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<CachedTokens> {
    let oauth_client = build_oauth_client(client_id, client_secret)?;
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
        client_id: client_id.to_string(),
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
