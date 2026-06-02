//! IAP-authenticated HTTP client for the REP orchestrator.
use crate::orchestrator::{
    DEFAULT_ORCHESTRATOR_URL, Error, GCP_PROJECT, IAP_OAUTH_CLIENT_ID_SECRET_NAME,
    IAP_OAUTH_CLIENT_SECRET_SECRET_NAME, Result,
    auth::{AdcCredentials, id_token},
};
use google_cloud_secretmanager_v1::client::SecretManagerService;
use reqwest::{Client, Method, RequestBuilder, Url};
use serde::{Serialize, de::DeserializeOwned};
use std::str::FromStr;
use tracing::debug;

/// Authenticated HTTP client for the REP orchestrator, protected by Google Cloud IAP.
#[derive(Debug)]
pub struct OrchestratorClient {
    adc: AdcCredentials,
    client_id: String,
    client_secret: String,
    http_client: Client,
    base_url: Url,
}

impl OrchestratorClient {
    /// Build a new client using the [default orchestrator URL][DEFAULT_ORCHESTRATOR_URL].
    ///
    /// Loads Application Default Credentials from disk and fetches the IAP
    /// OAuth client credentials from Secret Manager.
    pub async fn new() -> Result<Self> {
        Self::new_with_base_url(
            Url::from_str(DEFAULT_ORCHESTRATOR_URL).expect("default URL is valid"),
        )
        .await
    }

    /// Build a new client with a custom orchestrator URL.
    ///
    /// Loads Application Default Credentials from disk and fetches the IAP
    /// OAuth client credentials from Secret Manager.
    pub async fn new_with_base_url(base_url: Url) -> Result<Self> {
        let adc = AdcCredentials::load()?;

        let (client_id, client_secret) = fetch_secrets().await?;

        Ok(Self {
            adc,
            client_id,
            client_secret,
            http_client: Client::new(),
            base_url,
        })
    }

    /// Prepare a new authenticated request for sending to the orchestrator.
    pub async fn request(&self, method: Method, endpoint: &str) -> Result<RequestBuilder> {
        let id_token = id_token(&self.adc, &self.client_id, &self.client_secret).await?;
        let url = self
            .base_url
            .join(endpoint)
            .map_err(|e| Error::InvalidUrl(e.to_string()))?;

        Ok(self.http_client.request(method, url).bearer_auth(id_token))
    }

    /// Make a GET request to the orchestrator, deserializing the response body from JSON.
    pub async fn get_json<T>(&self, endpoint: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let resp = self.request(Method::GET, endpoint).await?.send().await?;
        let status = resp.status();

        if status.is_success() {
            Ok(resp.json().await?)
        } else {
            let body = resp.text().await?;

            Err(Error::FailedRequest { status, body })
        }
    }

    /// Make a JSON POST request to the orchestrator, deserializing the response body from JSON.
    pub async fn post_json<B, T>(&self, endpoint: &str, body: &B) -> Result<T>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let resp = self
            .request(Method::POST, endpoint)
            .await?
            .json(body)
            .send()
            .await?;

        let status = resp.status();

        if status.is_success() {
            Ok(resp.json().await?)
        } else {
            let body = resp.text().await?;

            Err(Error::FailedRequest { status, body })
        }
    }
}

async fn fetch_secrets() -> Result<(String, String)> {
    let client = SecretManagerService::builder()
        .build()
        .await
        .inspect_err(|e| debug!(error = ?e, "failed to build Secret Manager client"))?;

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
        .await
        .inspect_err(|e| debug!(error = ?e, secret_name, "Secret Manager send() failed"))?;

    let payload = response
        .payload
        .ok_or_else(|| Error::InvalidSecret("response missing payload".to_owned()))?;

    String::from_utf8(payload.data.to_vec()).map_err(|e| Error::InvalidSecret(e.to_string()))
}
