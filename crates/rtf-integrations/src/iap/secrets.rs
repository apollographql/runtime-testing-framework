//! Secret Manager helpers for fetching the IAP OAuth client credentials.

use crate::iap::{
    GCP_PROJECT, IAP_OAUTH_CLIENT_ID_SECRET_NAME, IAP_OAUTH_CLIENT_SECRET_SECRET_NAME,
    client::{Error, Result},
};
use google_cloud_secretmanager_v1::client::SecretManagerService;

/// The OAuth Web client credentials used both as the IAP audience and to drive
/// the CLI's user-consent flow.
pub(super) struct Secrets {
    pub client_id: String,
    pub client_secret: String,
}

/// Fetch the IAP OAuth client id + secret from Secret Manager.
///
/// The Secret Manager client uses Application Default Credentials internally;
/// users must have run `gcloud auth application-default login` at least once.
pub(super) async fn fetch_all() -> Result<Secrets> {
    let client = SecretManagerService::builder()
        .build()
        .await
        .map_err(|e| Error::Adc(e.to_string()))?;

    Ok(Secrets {
        client_id: fetch_secret(&client, IAP_OAUTH_CLIENT_ID_SECRET_NAME).await?,
        client_secret: fetch_secret(&client, IAP_OAUTH_CLIENT_SECRET_SECRET_NAME).await?,
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
