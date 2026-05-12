//! IAP-authenticated HTTP client for the REP orchestrator.

use crate::iap::{
    auth::{OauthConfig, access_token, id_token},
    secrets::{fetch_iap_audience, fetch_oauth_client_id, fetch_oauth_client_secret},
};
use bytes::Bytes;
use reqwest::{Client, Method};

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

/// Authenticated HTTP client for the REP orchestrator, protected by Google Cloud IAP.
///
/// Construct with [`IapClient::new`]; ADC loading and three Secret Manager
/// lookups (the IAP audience and the Desktop OAuth client ID / secret) are
/// performed eagerly during construction. The first call to [`IapClient::send`]
/// from a fresh user account opens a browser for OAuth consent; subsequent
/// invocations reuse a refresh token cached at `~/.config/rtf/iap_credentials.json`.
#[derive(Debug)]
pub struct IapClient {
    orchestrator_url: String,
    iap_audience: String,
    oauth_client_id: String,
    oauth_client_secret: String,
    http: Client,
}

impl IapClient {
    /// Build a new client.
    ///
    /// Loads ADC, obtains an access token, and fetches the IAP audience plus
    /// the Desktop OAuth client credentials from Secret Manager.
    pub async fn new(orchestrator_url: impl Into<String>) -> Result<Self> {
        let orchestrator_url = orchestrator_url.into();
        let access_token = access_token().await?;
        let iap_audience = fetch_iap_audience(&access_token).await?;
        let oauth_client_id = fetch_oauth_client_id(&access_token).await?;
        let oauth_client_secret = fetch_oauth_client_secret(&access_token).await?;
        let http = Client::new();
        Ok(Self {
            orchestrator_url,
            iap_audience,
            oauth_client_id,
            oauth_client_secret,
            http,
        })
    }

    /// Send an authenticated request to the orchestrator and return the full response.
    ///
    /// Mints a user-scoped Google-signed ID token aimed at the IAP audience and
    /// attaches it as a `Bearer` token. `method` is an HTTP method string
    /// (e.g. `"GET"`, `"POST"`).
    pub async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(String, String)],
        body: Option<Bytes>,
    ) -> Result<IapResponse> {
        let oauth_config = OauthConfig {
            iap_audience: &self.iap_audience,
            client_id: &self.oauth_client_id,
            client_secret: &self.oauth_client_secret,
        };
        let id_token = id_token(&oauth_config).await?;

        let base = self.orchestrator_url.trim_end_matches('/');
        let url = format!("{base}{path}");

        let parsed_method = method
            .parse::<Method>()
            .map_err(|e| Error::OauthFlow(format!("invalid HTTP method '{method}': {e}")))?;

        let mut builder = self
            .http
            .request(parsed_method, &url)
            .header("Authorization", format!("Bearer {id_token}"));

        for (key, value) in headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        if let Some(b) = body {
            builder = builder.body(b);
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
}
