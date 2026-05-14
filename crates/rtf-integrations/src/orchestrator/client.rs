//! IAP-authenticated HTTP client for the REP orchestrator.

use crate::orchestrator::{
    Error, GCP_PROJECT, IAP_OAUTH_CLIENT_ID_SECRET_NAME, IAP_OAUTH_CLIENT_SECRET_SECRET_NAME,
    Result,
    auth::{AdcCredentials, id_token},
};
use google_cloud_secretmanager_v1::client::SecretManagerService;
use oauth2::http::HeaderValue;
use reqwest::{Client, Request, Response, header::AUTHORIZATION};

/// Authenticated HTTP client for the REP orchestrator, protected by Google Cloud IAP.
#[derive(Debug)]
pub struct OrchestratorClient {
    adc: AdcCredentials,
    client_id: String,
    client_secret: String,
    http_client: Client,
}

impl OrchestratorClient {
    /// Build a new client.
    ///
    /// Loads Application Default Credentials from disk and fetches the IAP
    /// OAuth client credentials from Secret Manager.
    pub async fn new() -> Result<Self> {
        let adc = AdcCredentials::load()?;

        let (client_id, client_secret) = fetch_secrets().await?;

        Ok(Self {
            adc,
            client_id,
            client_secret,
            http_client: Client::new(),
        })
    }

    /// Send an authenticated request to the orchestrator and return the full response.
    pub async fn send(&self, mut request: Request) -> Result<Response> {
        let id_token = id_token(&self.adc, &self.client_id, &self.client_secret).await?;

        let mut auth_value = HeaderValue::from_str(&format!("Bearer {id_token}"))
            .map_err(Error::InvalidBearerToken)?;
        auth_value.set_sensitive(true);

        request.headers_mut().insert(AUTHORIZATION, auth_value);

        let response = self.http_client.execute(request).await?;

        Ok(response)
    }
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
