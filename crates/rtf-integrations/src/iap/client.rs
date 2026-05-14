//! IAP-authenticated HTTP client for the REP orchestrator.

use crate::iap::{
    AdcError, Error, GCP_PROJECT, IAP_OAUTH_CLIENT_ID_SECRET_NAME,
    IAP_OAUTH_CLIENT_SECRET_SECRET_NAME, Result, auth::id_token,
};
use google_cloud_secretmanager_v1::client::SecretManagerService;
use reqwest::{
    Client, Request, Response,
    header::{AUTHORIZATION, HeaderValue},
};
use std::{env, path::PathBuf};

/// Authenticated HTTP client for the REP orchestrator, protected by Google Cloud IAP.
#[derive(Debug)]
pub struct IapClient {
    client_id: String,
    client_secret: String,
    request: Request,
}

impl IapClient {
    /// Build a new client.
    ///
    /// Fetches the IAP OAuth client credentials from Secret Manager. The SDK
    /// loads Application Default Credentials internally.
    pub async fn new(request: Request) -> Result<Self> {
        require_adc()?;

        let (client_id, client_secret) = fetch_secrets().await?;

        Ok(Self {
            client_id,
            client_secret,
            request,
        })
    }

    /// Send an authenticated request to the orchestrator and return the full response.
    pub async fn send(self) -> Result<Response> {
        let id_token = id_token(&self.client_id, &self.client_secret).await?;

        let mut auth_value = HeaderValue::from_str(&format!("Bearer {id_token}"))
            .map_err(Error::InvalidBearerToken)?;
        auth_value.set_sensitive(true);

        let mut request = self.request;
        request.headers_mut().insert(AUTHORIZATION, auth_value);

        let response = Client::new().execute(request).await?;

        Ok(response)
    }
}

/// Fail fast with a clear message if no Application Default Credentials are configured.
///
/// Checks `$GOOGLE_APPLICATION_CREDENTIALS` first, then the well-known gcloud path. Either
/// presence is enough — actual validity is verified when the credential is used.
fn require_adc() -> Result<()> {
    let configured = env::var("GOOGLE_APPLICATION_CREDENTIALS").is_ok_and(|v| !v.is_empty())
        || env::var("HOME").is_ok_and(|home| {
            !home.is_empty()
                && PathBuf::from(home)
                    .join(".config/gcloud/application_default_credentials.json")
                    .exists()
        });

    if configured {
        return Ok(());
    }

    Err(AdcError::NotAuthenticated.into())
}

async fn fetch_secrets() -> Result<(String, String)> {
    let client = SecretManagerService::builder().build().await?;

    let client_id = fetch_secret(&client, IAP_OAUTH_CLIENT_ID_SECRET_NAME).await?;
    let client_secret = fetch_secret(&client, IAP_OAUTH_CLIENT_SECRET_SECRET_NAME).await?;

    Ok((client_id, client_secret))
}

async fn fetch_secret(client: &SecretManagerService, secret_name: &str) -> Result<String> {
    let response = client
        .access_secret_version()
        .set_name(format!(
            "projects/{GCP_PROJECT}/secrets/{secret_name}/versions/latest"
        ))
        .send()
        .await?;

    let payload = response
        .payload
        .ok_or_else(|| Error::InvalidSecret("response missing payload".to_owned()))?;

    String::from_utf8(payload.data.to_vec()).map_err(|e| Error::InvalidSecret(e.to_string()))
}
