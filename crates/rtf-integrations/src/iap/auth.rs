//! GCP access tokens (for Secret Manager) and the user-OAuth loopback flow
//! that mints IAP-audience ID tokens.

use crate::iap::client::{Error, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use google_cloud_auth::credentials::Builder;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::{env, fs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const OAUTH_SCOPES: &str = "openid email";
const ACCESS_TOKEN_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const TOKEN_CACHE_SUBDIR: &str = "rtf";
const TOKEN_CACHE_FILENAME: &str = "iap_credentials.json";
const LOOPBACK_TIMEOUT_SECS: u64 = 300;
const ID_TOKEN_EXPIRY_SKEW_SECS: i64 = 60;

/// Inputs for the user-OAuth flow: the IAP audience to target and the Desktop
/// OAuth client credentials that drive user consent.
pub(super) struct OauthConfig<'a> {
    pub iap_audience: &'a str,
    pub client_id: &'a str,
    pub client_secret: &'a str,
}

/// Persisted token state at `~/.config/rtf/iap_credentials.json`.
#[derive(Debug, Serialize, Deserialize)]
struct CachedTokens {
    audience: String,
    client_id: String,
    id_token: String,
    id_token_expires_at: DateTime<Utc>,
    refresh_token: String,
}

/// Obtain a GCP access token from Application Default Credentials.
///
/// Used to authenticate the Secret Manager bootstrap calls that fetch the IAP
/// audience and the Desktop OAuth client credentials. Requires the user to
/// have run `gcloud auth application-default login` at least once.
pub(super) async fn access_token() -> Result<String> {
    let credentials = Builder::default()
        .with_scopes([ACCESS_TOKEN_SCOPE])
        .build_access_token_credentials()
        .map_err(|e| Error::Adc(e.to_string()))?;
    let token = credentials
        .access_token()
        .await
        .map_err(|e| Error::Adc(e.to_string()))?;

    Ok(token.token)
}

/// Obtain an IAP-audience OIDC ID token authenticated as the current user.
///
/// Tries the on-disk cache, then a refresh-token grant, then runs the full
/// loopback OAuth flow (which opens a browser).
pub(super) async fn id_token(config: &OauthConfig<'_>) -> Result<String> {
    if let Some(cached) = load_cache()
        && cached.audience == config.iap_audience
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

/// Token endpoint response shape, common to both authorization-code and
/// refresh-token grants.
#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

async fn run_oauth_flow(config: &OauthConfig<'_>) -> Result<CachedTokens> {
    let code_verifier = generate_code_verifier();
    let code_challenge = compute_code_challenge(&code_verifier);
    let state = random_url_safe(24);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::OauthFlow(format!("could not bind loopback listener: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::OauthFlow(format!("could not read loopback port: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let auth_url = build_auth_url(config.client_id, &redirect_uri, &code_challenge, &state);

    eprintln!("Opening browser to authenticate with IAP…");
    eprintln!("If the browser does not open, visit this URL:\n  {auth_url}\n");
    let _ = open::that(&auth_url);

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

    if returned_state != state {
        return Err(Error::OauthFlow(
            "OAuth state mismatch — aborting to prevent CSRF".to_owned(),
        ));
    }

    let response = exchange_code(&code, &code_verifier, &redirect_uri, config).await?;
    let refresh_token = response.refresh_token.ok_or_else(|| {
        Error::OauthFlow(
            "Google did not return a refresh_token; check that the OAuth consent screen \
             grants offline access"
                .to_owned(),
        )
    })?;

    Ok(CachedTokens {
        audience: config.iap_audience.to_owned(),
        client_id: config.client_id.to_owned(),
        id_token: response.id_token,
        id_token_expires_at: Utc::now() + Duration::seconds(response.expires_in),
        refresh_token,
    })
}

fn build_auth_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let params = [
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", OAUTH_SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    format!("{GOOGLE_AUTH_ENDPOINT}?{}", encode_form(&params))
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
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            let value = percent_decode(v);
            match k {
                "code" => code = Some(value),
                "state" => state = Some(value),
                "error" => oauth_error = Some(value),
                _ => {}
            }
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

async fn exchange_code(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    config: &OauthConfig<'_>,
) -> Result<TokenResponse> {
    let params = [
        ("client_id", config.client_id),
        ("client_secret", config.client_secret),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
        ("audience", config.iap_audience),
    ];

    post_token_request(&params).await
}

async fn refresh_id_token(refresh_token: &str, config: &OauthConfig<'_>) -> Result<CachedTokens> {
    let params = [
        ("client_id", config.client_id),
        ("client_secret", config.client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
        ("audience", config.iap_audience),
    ];
    let resp = post_token_request(&params).await?;

    Ok(CachedTokens {
        audience: config.iap_audience.to_owned(),
        client_id: config.client_id.to_owned(),
        id_token: resp.id_token,
        id_token_expires_at: Utc::now() + Duration::seconds(resp.expires_in),
        // Refresh-token grants may or may not return a new refresh_token; keep
        // the existing one if Google didn't rotate it.
        refresh_token: resp
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_owned()),
    })
}

async fn post_token_request(params: &[(&str, &str)]) -> Result<TokenResponse> {
    let client = Client::new();
    let response = client
        .post(GOOGLE_TOKEN_ENDPOINT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(encode_form(params))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::OauthFlow(format!(
            "Google token endpoint returned HTTP {status}: {body}"
        )));
    }

    response.json().await.map_err(Error::Http)
}

// ── PKCE + URL-safe random helpers ─────────────────────────────────────────

fn generate_code_verifier() -> String {
    // RFC 7636 §4.1: code_verifier is a high-entropy string of 43–128 chars
    // from the unreserved URL-safe alphabet. 32 random bytes → 43 base64url chars.
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);

    URL_SAFE_NO_PAD.encode(bytes)
}

fn compute_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());

    URL_SAFE_NO_PAD.encode(digest)
}

fn random_url_safe(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    rand::fill(bytes.as_mut_slice());

    URL_SAFE_NO_PAD.encode(bytes)
}

fn encode_form(params: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        url_encode_into(&mut out, k);
        out.push('=');
        url_encode_into(&mut out, v);
    }

    out
}

fn url_encode_into(out: &mut String, s: &str) {
    // application/x-www-form-urlencoded per WHATWG: alnum and `*-._` pass
    // through; space → `+`; everything else → percent-encoded UTF-8 bytes.
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(hex_char(byte >> 4));
                out.push(hex_char(byte & 0x0F));
            }
        }
    }
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + nibble - 10) as char,
        _ => unreachable!(),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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

    #[test]
    fn code_verifier_is_43_chars_url_safe() {
        let v = generate_code_verifier();
        assert_eq!(v.len(), 43, "RFC 7636: 32 bytes → 43 base64url chars");
        assert!(
            v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "verifier must be URL-safe: {v}"
        );
    }

    #[test]
    fn code_challenge_is_base64url_sha256_of_verifier() {
        // Test vector from RFC 7636 §4.6.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(compute_code_challenge(verifier), expected);
    }

    #[test]
    fn percent_decode_handles_plus_and_hex() {
        assert_eq!(percent_decode("hello+world"), "hello world");
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%2F%3A"), "/:");
        assert_eq!(percent_decode("noop"), "noop");
    }

    #[test]
    fn percent_decode_passes_through_malformed_percent() {
        assert_eq!(percent_decode("%ZZ"), "%ZZ");
    }

    #[test]
    fn html_escape_escapes_dangerous_chars() {
        assert_eq!(
            html_escape("<script>alert(\"x\")</script>"),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn build_auth_url_contains_expected_params() {
        let url = build_auth_url("CID", "http://127.0.0.1:1234", "CHAL", "ST");
        assert!(url.starts_with(GOOGLE_AUTH_ENDPOINT));
        assert!(url.contains("client_id=CID"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1234"));
        assert!(url.contains("code_challenge=CHAL"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=ST"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("scope=openid+email"));
    }
}
