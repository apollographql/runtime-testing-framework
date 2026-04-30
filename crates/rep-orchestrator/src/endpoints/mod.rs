//! Request handlers for the orchestrator axum server
use crate::Error;
use axum::{extract::FromRequestParts, http::request::Parts};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use tracing::warn;
use uuid::Uuid;

pub mod execution_artifacts;
pub mod execution_config;
pub mod execution_status;
pub mod generate_upload_urls;
pub mod health;
pub mod run_status;
pub mod trigger;

/// Axum extractor that parses the `Authorization: Bearer <token>` header.
///
/// The extracted token should be validated against the per-execution token stored in the
/// database using [BearerToken::verify].
pub struct BearerToken(String);

impl BearerToken {
    pub fn verify(&self, expected: Uuid) -> crate::Result<()> {
        let provided: Uuid = self.0.parse().map_err(|_| {
            warn!("Provided bearer token is not a valid UUID, returning 403");
            Error::Unauthorized
        })?;
        if provided == expected {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }
}

impl<S> FromRequestParts<S> for BearerToken
where
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(bearer)) =
            TypedHeader::<Authorization<Bearer>>::from_request_parts(parts, state)
                .await
                .map_err(|err| {
                    warn!(%err, "Failed to parse bearer token, returning 403");
                    Error::Unauthorized
                })?;

        Ok(Self(bearer.token().to_owned()))
    }
}
