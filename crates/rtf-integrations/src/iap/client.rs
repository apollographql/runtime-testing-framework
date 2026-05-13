//! IAP-authenticated HTTP client for the REP orchestrator.

use crate::iap::{
    auth::{OauthConfig, id_token},
    secrets::{Secrets, fetch_all},
};
use bytes::Bytes;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Body, Client, Method};
use std::pin::Pin;
use std::str::FromStr;
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

/// Errors that can occur when building or using an [`IapClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Application Default Credentials could not be loaded or used.
    ///
    /// Most commonly resolved by running `gcloud auth application-default login`.
    #[error("GCP authentication error: {0}")]
    Adc(String),

    /// The interactive user OAuth flow failed (browser, loopback server,
    /// state mismatch, or the Google token endpoint rejected the request).
    #[error("user OAuth flow failed: {0}")]
    OauthFlow(String),

    /// The on-disk token cache could not be read or written.
    #[error("token cache I/O error: {0}")]
    TokenCache(#[source] std::io::Error),

    /// The Secret Manager REST endpoint returned HTTP 403.
    ///
    /// Grant the `secretmanager.secretAccessor` role on the project, or contact
    /// the Runtime Readiness team.
    #[error(
        "Secret Manager access denied — ask the Runtime Readiness team to grant secretmanager.secretAccessor on project"
    )]
    SecretManagerForbidden,

    /// The Secret Manager REST endpoint returned a non-2xx status other than 403.
    #[error("Secret Manager request failed (HTTP {status}): {body}")]
    SecretManagerHttp {
        /// The HTTP status code returned by Secret Manager.
        status: u16,
        /// The response body returned by Secret Manager.
        body: String,
    },

    /// The orchestrator returned HTTP 401 or 403, indicating an IAP access problem.
    #[error(
        "IAP access denied — your account is authenticated but not authorized; \
         ask the Runtime Readiness team to add you to the IAP allowlist"
    )]
    IapForbidden,

    /// An underlying HTTP transport error.
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    /// The secret payload returned by Secret Manager could not be decoded.
    #[error("could not decode secret payload: {0}")]
    InvalidSecret(String),

    /// The caller passed an unparsable HTTP method string.
    #[error("invalid HTTP method {method:?}: {reason}")]
    InvalidMethod {
        /// The original method string the caller supplied.
        method: String,
        /// The parser's error message.
        reason: String,
    },

    /// The caller passed a header string that does not match `Key: Value`.
    #[error("invalid header {raw:?}: {reason}")]
    InvalidHeader {
        /// The original header string the caller supplied.
        raw: String,
        /// The parser's error message.
        reason: String,
    },
}

/// Alias for a [Result][std::result::Result] where the error variant is an [Error].
pub type Result<T> = std::result::Result<T, Error>;

/// The completed response from a single orchestrator request.
#[derive(Debug)]
pub struct IapResponse {
    /// Raw HTTP status code.
    pub status: u16,
    /// Full response body.
    pub body: Bytes,
}

impl IapResponse {
    /// Returns `true` when the status is in the 2xx range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Body for an outgoing IAP request.
///
/// Construct with [`RequestBody::from_bytes`] for in-memory payloads, or
/// [`RequestBody::from_reader`] for streaming sources (open files, stdin, etc).
/// Streaming bodies flow through the HTTP client chunk-by-chunk and never
/// accumulate in memory, so multi-gigabyte payloads are safe.
pub struct RequestBody {
    inner: RequestBodyInner,
}

enum RequestBodyInner {
    Bytes(Vec<u8>),
    Reader(Pin<Box<dyn AsyncRead + Send + 'static>>),
}

impl RequestBody {
    /// Build a body from a fixed byte buffer. Sent with `Content-Length`.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            inner: RequestBodyInner::Bytes(bytes.into()),
        }
    }

    /// Build a body from a streaming reader. Sent with chunked transfer encoding.
    pub fn from_reader<R>(reader: R) -> Self
    where
        R: AsyncRead + Send + 'static,
    {
        Self {
            inner: RequestBodyInner::Reader(Box::pin(reader)),
        }
    }
}

impl std::fmt::Debug for RequestBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            RequestBodyInner::Bytes(b) => f
                .debug_struct("RequestBody")
                .field("kind", &"Bytes")
                .field("len", &b.len())
                .finish(),
            RequestBodyInner::Reader(_) => f
                .debug_struct("RequestBody")
                .field("kind", &"Reader")
                .finish(),
        }
    }
}

impl From<RequestBody> for Body {
    fn from(body: RequestBody) -> Self {
        match body.inner {
            RequestBodyInner::Bytes(b) => Body::from(b),
            RequestBodyInner::Reader(r) => Body::wrap_stream(ReaderStream::new(r)),
        }
    }
}

/// Authenticated HTTP client for the REP orchestrator, protected by Google Cloud IAP.
///
/// Construct with [`IapClient::new`]; ADC loading and two Secret Manager
/// lookups (the IAP OAuth client id and secret) are performed eagerly during
/// construction. The first call to [`IapClient::send`] from a fresh user
/// account opens a browser for OAuth consent; subsequent invocations reuse a
/// refresh token cached at `~/.config/rtf/iap_credentials.json`.
#[derive(Debug)]
pub struct IapClient {
    orchestrator_url: String,
    client_id: String,
    client_secret: String,
    http: Client,
}

impl IapClient {
    /// Build a new client.
    ///
    /// Fetches the IAP OAuth client credentials from Secret Manager. The SDK
    /// loads Application Default Credentials internally.
    pub async fn new(orchestrator_url: impl Into<String>) -> Result<Self> {
        let orchestrator_url = orchestrator_url.into();
        let Secrets {
            client_id,
            client_secret,
        } = fetch_all().await?;
        let http = Client::new();
        Ok(Self {
            orchestrator_url,
            client_id,
            client_secret,
            http,
        })
    }

    /// Send an authenticated request to the orchestrator and return the full response.
    ///
    /// Mints a user-scoped Google-signed ID token aimed at the IAP audience and
    /// attaches it as a `Bearer` token. Each entry in `headers` must be a
    /// `"Key: Value"` string; parse failures surface as [`Error::InvalidHeader`].
    /// The supplied headers cannot override `Authorization`.
    pub async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[String],
        body: Option<RequestBody>,
    ) -> Result<IapResponse> {
        let method = parse_method(method)?;
        let header_map = parse_headers(headers)?;

        let oauth_config = OauthConfig {
            client_id: &self.client_id,
            client_secret: &self.client_secret,
        };
        let id_token = id_token(&oauth_config).await?;

        let base = self.orchestrator_url.trim_end_matches('/');
        let url = format!("{base}{path}");

        let mut auth_value = HeaderValue::from_str(&format!("Bearer {id_token}"))
            .map_err(|e| Error::OauthFlow(format!("invalid bearer token: {e}")))?;
        auth_value.set_sensitive(true);

        let mut builder = self.http.request(method, &url).headers(header_map);
        builder = builder.header(AUTHORIZATION, auth_value);

        if let Some(b) = body {
            builder = builder.body(Body::from(b));
        }

        let response = builder.send().await?;
        let status = response.status().as_u16();

        if status == 401 || status == 403 {
            return Err(Error::IapForbidden);
        }

        let body = response.bytes().await?;

        Ok(IapResponse { status, body })
    }
}

fn parse_method(method: &str) -> Result<Method> {
    Method::from_str(method).map_err(|e| Error::InvalidMethod {
        method: method.to_owned(),
        reason: e.to_string(),
    })
}

fn parse_headers(headers: &[String]) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for raw in headers {
        let (name, value) = raw.split_once(':').ok_or_else(|| Error::InvalidHeader {
            raw: raw.clone(),
            reason: "expected \"Key: Value\"".to_owned(),
        })?;
        let name = HeaderName::from_str(name.trim()).map_err(|e| Error::InvalidHeader {
            raw: raw.clone(),
            reason: e.to_string(),
        })?;
        let value =
            HeaderValue::from_str(value.trim_start()).map_err(|e| Error::InvalidHeader {
                raw: raw.clone(),
                reason: e.to_string(),
            })?;
        map.append(name, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_manager_forbidden_names_iam_role_and_team() {
        let msg = Error::SecretManagerForbidden.to_string();
        assert!(
            msg.contains("secretmanager.secretAccessor"),
            "expected IAM role name in message, got: {msg}"
        );
        assert!(
            msg.contains("Runtime Readiness team"),
            "expected Runtime Readiness team suggestion in message, got: {msg}"
        );
    }

    #[test]
    fn iap_forbidden_message_blames_authorization_not_authentication() {
        let msg = Error::IapForbidden.to_string();
        assert!(
            msg.contains("IAP allowlist"),
            "expected IAP allowlist hint in message, got: {msg}"
        );
        assert!(
            msg.contains("Runtime Readiness team"),
            "expected Runtime Readiness team escalation in message, got: {msg}"
        );
    }

    #[test]
    fn iap_response_is_success_covers_2xx_range() {
        for status in [200u16, 201, 204, 299] {
            let resp = IapResponse {
                status,
                body: Bytes::new(),
            };
            assert!(
                resp.is_success(),
                "expected is_success() for status {status}"
            );
        }
        for status in [199u16, 300, 400, 401, 403, 404, 500] {
            let resp = IapResponse {
                status,
                body: Bytes::new(),
            };
            assert!(
                !resp.is_success(),
                "expected !is_success() for status {status}"
            );
        }
    }

    #[test]
    fn parse_method_accepts_known_verbs() {
        assert_eq!(parse_method("GET").unwrap(), Method::GET);
        assert_eq!(parse_method("POST").unwrap(), Method::POST);
        assert_eq!(parse_method("PATCH").unwrap(), Method::PATCH);
    }

    #[test]
    fn parse_method_rejects_garbage() {
        let err = parse_method("GET POST").unwrap_err();
        assert!(matches!(err, Error::InvalidMethod { .. }), "got {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("GET POST"));
    }

    #[test]
    fn parse_headers_builds_header_map() {
        let headers = vec![
            "Content-Type: application/json".to_owned(),
            "X-Custom: foo: bar".to_owned(),
        ];
        let map = parse_headers(&headers).unwrap();
        assert_eq!(
            map.get("content-type").unwrap().to_str().unwrap(),
            "application/json"
        );
        assert_eq!(map.get("x-custom").unwrap().to_str().unwrap(), "foo: bar");
    }

    #[test]
    fn parse_headers_rejects_missing_colon() {
        let headers = vec!["BearerToken".to_owned()];
        let err = parse_headers(&headers).unwrap_err();
        assert!(matches!(err, Error::InvalidHeader { .. }), "got {err:?}");
    }
}
