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
mod secrets;

pub use client::{Error, IapClient, IapResponse, RequestBody};

/// The GCP project that hosts the REP orchestrator's Secret Manager secrets.
pub const GCP_PROJECT: &str = "runtime-testing-framework";

/// The Secret Manager secret name storing the IAP OAuth client ID.
///
/// This is the Web OAuth client that IAP is configured with — its ID is both
/// the audience IAP validates against and the `client_id` the CLI uses to
/// drive the user-consent loopback flow. `http://localhost` must be in the
/// client's authorized redirect URIs.
pub const IAP_OAUTH_CLIENT_ID_SECRET_NAME: &str = "iap-orchestrator-client-id";

/// The Secret Manager secret name storing the IAP OAuth client secret.
pub const IAP_OAUTH_CLIENT_SECRET_SECRET_NAME: &str = "iap-orchestrator-client-secret";

/// Default base URL for the REP orchestrator.
pub const DEFAULT_ORCHESTRATOR_URL: &str = "https://api.rtf.apollographql.com";

/// Environment variable that overrides [`DEFAULT_ORCHESTRATOR_URL`] at runtime.
///
/// If set, its value is used as the orchestrator base URL unless `--orchestrator-url`
/// is also provided on the command line, which takes precedence.
pub const ORCHESTRATOR_URL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_URL";
