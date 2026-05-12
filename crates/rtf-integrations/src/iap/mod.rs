//! IAP-authenticated HTTP client for the REP orchestrator.
//!
//! This module provides everything needed to make authenticated HTTP requests
//! to the REP orchestrator service, which is protected by Google Cloud IAP.
//! Authentication uses an interactive user OAuth loopback flow: the CLI opens a
//! browser, the user consents, and the resulting refresh token is cached on
//! disk. The IAP audience and the Desktop OAuth client credentials are fetched
//! at runtime from Secret Manager (using the user's ADC access token to
//! bootstrap).

mod auth;
mod client;
mod secrets;

pub use client::{Error, IapClient, IapResponse, RequestBody};

/// The GCP project that hosts the REP orchestrator and its Secret Manager secrets.
pub const GCP_PROJECT: &str = "runtime-testing-framework";

/// The Secret Manager secret name storing the IAP OAuth client ID (audience).
pub const IAP_SECRET_NAME: &str = "iap-orchestrator-client-id";

/// The Secret Manager secret name storing the Desktop OAuth client ID that the
/// CLI uses to drive the user-consent loopback flow.
pub const OAUTH_CLIENT_ID_SECRET_NAME: &str = "rtf-cli-client-id";

/// The Secret Manager secret name storing the Desktop OAuth client secret.
///
/// Per RFC 8252 §8.5 this value is not actually confidential for an installed
/// app; centralizing it in Secret Manager only eases rotation.
pub const OAUTH_CLIENT_SECRET_SECRET_NAME: &str = "rtf-cli-client-secret";

/// Default base URL for the REP orchestrator.
pub const DEFAULT_ORCHESTRATOR_URL: &str = "https://api.rtf.apollographql.com";

/// Environment variable that overrides [`DEFAULT_ORCHESTRATOR_URL`] at runtime.
///
/// If set, its value is used as the orchestrator base URL unless `--orchestrator-url`
/// is also provided on the command line, which takes precedence.
pub const ORCHESTRATOR_URL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_URL";
