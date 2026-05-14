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

pub use client::OrchestratorClient;

/// The GCP project that hosts the REP orchestrator's Secret Manager secrets.
const GCP_PROJECT: &str = "runtime-testing-framework";

/// The Secret Manager secret name storing the IAP OAuth client ID.
///
/// This is the Web OAuth client that IAP is configured with — its ID is both
/// the audience IAP validates against and the `client_id` the CLI uses to
/// drive the user-consent loopback flow. `http://localhost` must be in the
/// client's authorized redirect URIs.
const IAP_OAUTH_CLIENT_ID_SECRET_NAME: &str = "iap-orchestrator-client-id";

/// The Secret Manager secret name storing the IAP OAuth client secret.
const IAP_OAUTH_CLIENT_SECRET_SECRET_NAME: &str = "iap-orchestrator-client-secret";

/// Errors that can occur when building or using an [`OrchestratorClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The secret payload returned by Secret Manager could not be decoded.
    #[error("could not decode secret payload: {0}")]
    InvalidSecret(String),

    /// The interactive user OAuth flow failed.
    #[error(transparent)]
    OauthFlow(#[from] OauthError),

    /// Application Default Credentials error.
    #[error(transparent)]
    Adc(#[from] AdcError),

    /// The on-disk token cache could not be read or written.
    #[error("token cache error: {0}")]
    TokenCache(#[from] std::io::Error),

    /// The ID token could not be formatted as an HTTP Authorization header value.
    #[error("invalid bearer token: {0}")]
    InvalidBearerToken(#[source] reqwest::header::InvalidHeaderValue),

    /// An error from the Google Cloud client builder.
    #[error(transparent)]
    GoogleClientError(#[from] google_cloud_gax::client_builder::Error),

    /// An error from the Google Secret Manager API.
    #[error(transparent)]
    GoogleSecretManager(#[from] google_cloud_secretmanager_v1::Error),

    /// An HTTP transport error.
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    /// A serde json error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Alias for a [Result][std::result::Result] where the error variant is an [Error].
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from Application Default Credentials.
#[derive(Debug, thiserror::Error)]
pub enum AdcError {
    /// No GCP credentials found — the user needs to run `gcloud auth application-default login`.
    #[error("not authenticated to GCP — run `gcloud auth application-default login` first")]
    NotAuthenticated,

    /// The home directory cannot be determined, so the ADC file path is unknown.
    #[error("unable to find home directory")]
    NoHomeDir,

    /// The ADC credentials file could not be read from disk.
    #[error("could not read ADC credentials file: {0}")]
    ReadFailed(#[source] std::io::Error),

    /// The ADC credentials file could not be parsed as JSON.
    #[error("could not parse ADC credentials file: {0}")]
    ParseFailed(#[source] serde_json::Error),

    /// The service-account ID token credentials object could not be built.
    #[error("could not build service-account ID token credentials: {0}")]
    CredentialsBuild(String),

    /// The service-account ID token could not be minted.
    #[error("could not mint service-account ID token: {0}")]
    TokenMint(String),
}
/// Errors from the interactive user OAuth loopback flow.
#[derive(Debug, thiserror::Error)]
pub enum OauthError {
    /// The token cache path cannot be determined because both `HOME` and `XDG_CONFIG_HOME` are
    /// unset.
    #[error("cannot determine token cache location (HOME and XDG_CONFIG_HOME both unset)")]
    NoCacheLocation,

    /// A Google OAuth/token endpoint URL constant failed to parse.
    #[error("invalid {endpoint} URL: {message}")]
    InvalidUrl {
        /// Which endpoint (`"auth"` or `"token"`).
        endpoint: &'static str,
        /// The underlying parse error message.
        message: String,
    },

    /// The reqwest client used for token-endpoint calls could not be built.
    #[error("could not build OAuth HTTP client: {0}")]
    HttpClientBuild(String),

    /// The loopback TCP listener could not be bound.
    #[error("could not bind loopback listener: {0}")]
    LoopbackBind(#[source] std::io::Error),

    /// The loopback port could not be read from the bound listener.
    #[error("could not read loopback port: {0}")]
    LoopbackPort(#[source] std::io::Error),

    /// The computed redirect URI failed to parse.
    #[error("invalid redirect URI: {0}")]
    InvalidRedirectUri(String),

    /// The browser did not complete the redirect within the timeout window.
    #[error("OAuth flow timed out after {0}s without a redirect")]
    FlowTimeout(u64),

    /// The `state` returned by Google did not match the issued CSRF token.
    #[error("OAuth state mismatch — aborting to prevent CSRF")]
    CsrfMismatch,

    /// Google's token endpoint rejected the code exchange or refresh grant.
    ///
    /// Stringified because `oauth2`'s `RequestTokenError` carries unwieldy generic parameters.
    #[error("Google token endpoint error: {0}")]
    TokenEndpoint(String),

    /// Google completed the flow but did not return a `refresh_token`.
    #[error(
        "Google did not return a refresh_token; check that the OAuth consent screen grants \
         offline access"
    )]
    NoRefreshToken,

    /// The loopback TCP accept call failed.
    #[error("loopback accept failed: {0}")]
    LoopbackAccept(#[source] std::io::Error),

    /// Reading from the loopback TCP stream failed.
    #[error("loopback read failed: {0}")]
    LoopbackRead(#[source] std::io::Error),

    /// The HTTP request received on the loopback port was malformed.
    #[error("malformed OAuth callback: {0}")]
    MalformedCallback(&'static str),

    /// The authorization server returned an `error` parameter in the callback.
    #[error("authorization denied: {0}")]
    AuthorizationDenied(String),

    /// The OAuth callback did not include a `code` parameter.
    #[error("missing code in OAuth callback")]
    MissingCode,

    /// The OAuth callback did not include a `state` parameter.
    #[error("missing state in OAuth callback")]
    MissingState,
}
