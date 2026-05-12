//! Secret Manager helpers for fetching IAP and OAuth-client configuration.

use crate::iap::{
    GCP_PROJECT, IAP_SECRET_NAME, OAUTH_CLIENT_ID_SECRET_NAME, OAUTH_CLIENT_SECRET_SECRET_NAME,
    client::{Error, Result},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::Client;

const SECRET_MANAGER_URL: &str = "https://secretmanager.googleapis.com/v1";

/// Fetch a Secret Manager secret payload as a UTF-8 string.
///
/// Reads `projects/{GCP_PROJECT}/secrets/{secret_name}/versions/latest` using
/// the provided GCP access token.
async fn fetch_secret(access_token: &str, secret_name: &str) -> Result<String> {
    let url = format!(
        "{SECRET_MANAGER_URL}/projects/{GCP_PROJECT}/secrets/{secret_name}/versions/latest:access"
    );

    let client = Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await?;

    let status = response.status();
    let status_u16 = status.as_u16();

    if status_u16 == 403 {
        return Err(Error::SecretManagerForbidden);
    }

    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "(unavailable)".to_owned());
        return Err(Error::SecretManagerHttp {
            status: status_u16,
            body,
        });
    }

    let json: serde_json::Value = response.json().await?;
    let encoded = json
        .get("payload")
        .and_then(|p| p.get("data"))
        .and_then(|d| d.as_str())
        .ok_or_else(|| Error::InvalidSecret("missing payload.data field".to_owned()))?;

    let bytes = STANDARD
        .decode(encoded)
        .map_err(|e| Error::InvalidSecret(e.to_string()))?;

    String::from_utf8(bytes).map_err(|e| Error::InvalidSecret(e.to_string()))
}

/// Fetch the IAP audience (OAuth client ID) for the REP orchestrator.
pub(super) async fn fetch_iap_audience(access_token: &str) -> Result<String> {
    fetch_secret(access_token, IAP_SECRET_NAME).await
}

/// Fetch the Desktop OAuth client ID that drives the user-consent loopback flow.
pub(super) async fn fetch_oauth_client_id(access_token: &str) -> Result<String> {
    fetch_secret(access_token, OAUTH_CLIENT_ID_SECRET_NAME).await
}

/// Fetch the Desktop OAuth client secret.
pub(super) async fn fetch_oauth_client_secret(access_token: &str) -> Result<String> {
    fetch_secret(access_token, OAUTH_CLIENT_SECRET_SECRET_NAME).await
}
