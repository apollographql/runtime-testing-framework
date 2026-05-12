//! Secret Manager helpers for fetching IAP and OAuth-client configuration.

use crate::iap::{
    GCP_PROJECT, IAP_SECRET_NAME, OAUTH_CLIENT_ID_SECRET_NAME, OAUTH_CLIENT_SECRET_SECRET_NAME,
    client::{Error, Result},
};
use google_cloud_secretmanager_v1::client::SecretManagerService;

/// The three bootstrap secrets needed to authenticate to the orchestrator.
pub(super) struct Secrets {
    pub iap_audience: String,
    pub oauth_client_id: String,
    pub oauth_client_secret: String,
}

/// Fetch all three IAP/OAuth bootstrap secrets in sequence.
///
/// The Secret Manager client uses Application Default Credentials internally;
/// users must have run `gcloud auth application-default login` at least once.
pub(super) async fn fetch_all() -> Result<Secrets> {
    let client = SecretManagerService::builder()
        .build()
        .await
        .map_err(|e| Error::Adc(e.to_string()))?;

    Ok(Secrets {
        iap_audience: fetch_secret(&client, IAP_SECRET_NAME).await?,
        oauth_client_id: fetch_secret(&client, OAUTH_CLIENT_ID_SECRET_NAME).await?,
        oauth_client_secret: fetch_secret(&client, OAUTH_CLIENT_SECRET_SECRET_NAME).await?,
    })
}

/// Fetch a Secret Manager secret payload as a UTF-8 string.
async fn fetch_secret(client: &SecretManagerService, secret_name: &str) -> Result<String> {
    let response = client
        .access_secret_version()
        .set_name(format!(
            "projects/{GCP_PROJECT}/secrets/{secret_name}/versions/latest"
        ))
        .send()
        .await
        .map_err(map_sdk_error)?;

    let payload = response
        .payload
        .ok_or_else(|| Error::InvalidSecret("response missing payload".to_owned()))?;

    String::from_utf8(payload.data.to_vec()).map_err(|e| Error::InvalidSecret(e.to_string()))
}

/// Map a Secret Manager SDK error into our local [`Error`] enum.
///
/// 403 is special-cased so callers get a precise IAM-role hint; other HTTP
/// statuses surface verbatim; non-HTTP failures (network, auth) land in
/// [`Error::Adc`].
fn map_sdk_error(err: google_cloud_gax::error::Error) -> Error {
    match err.http_status_code() {
        Some(403) => Error::SecretManagerForbidden,
        Some(status) => Error::SecretManagerHttp {
            status,
            body: err.to_string(),
        },
        None => Error::Adc(err.to_string()),
    }
}
