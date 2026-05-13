//! IAP-authenticated HTTP client for the REP orchestrator.
//!
//! This module provides everything needed to make authenticated HTTP requests
//! to the REP orchestrator service, which is protected by Google Cloud IAP.
//! Authentication uses an interactive user OAuth loopback flow against the
//! same Web OAuth client that IAP itself is configured with: the CLI opens a
//! browser, the user consents, and the resulting refresh token is cached on
//! disk. The Web client's id and secret are fetched at runtime from Secret
//! Manager (using the user's ADC access token to bootstrap).

mod auth;
mod client;

pub use client::IapClient;

/// The GCP project that hosts the REP orchestrator's Secret Manager secrets.
pub(crate) const GCP_PROJECT: &str = "runtime-testing-framework";

/// The Secret Manager secret name storing the IAP OAuth client ID.
///
/// This is the Web OAuth client that IAP is configured with — its ID is both
/// the audience IAP validates against and the `client_id` the CLI uses to
/// drive the user-consent loopback flow. `http://localhost` must be in the
/// client's authorized redirect URIs.
pub(crate) const IAP_OAUTH_CLIENT_ID_SECRET_NAME: &str = "iap-orchestrator-client-id";

/// The Secret Manager secret name storing the IAP OAuth client secret.
pub(crate) const IAP_OAUTH_CLIENT_SECRET_SECRET_NAME: &str = "iap-orchestrator-client-secret";

/// Errors that can occur when building or using an [`IapClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The secret payload returned by Secret Manager could not be decoded.
    #[error("could not decode secret payload: {0}")]
    InvalidSecret(String),

    /// The interactive user OAuth flow failed
    #[error("user OAuth flow failed: {0}")]
    OauthFlow(String),

    /// Application Default Credentials could not mint a service-account ID token.
    #[error("ADC error: {0}")]
    Adc(String),

    /// The on-disk token cache could not be read or written.
    #[error("token cache error: {0}")]
    TokenCache(#[from] std::io::Error),

    /// An error from the Google Cloud client builder.
    #[error(transparent)]
    GoogleClientError(#[from] google_cloud_gax::client_builder::Error),

    /// An error from the Google Secret Manager API.
    #[error(transparent)]
    GoogleSecretManager(#[from] google_cloud_secretmanager_v1::Error),

    /// An HTTP transport error.
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    /// A serde json error
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Alias for a [Result][std::result::Result] where the error variant is an [Error].
pub type Result<T> = std::result::Result<T, Error>;
