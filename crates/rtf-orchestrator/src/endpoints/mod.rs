//! Request handlers for the orchestrator axum server
use crate::{
    Error,
    state::{ServerState, UserType},
};
use axum::{extract::FromRequestParts, http::request::Parts};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use tracing::warn;
use uuid::Uuid;

pub mod admin;
pub mod execution_artifacts;
pub mod execution_config;
pub mod execution_status;
pub mod generate_upload_urls;
pub mod health;
pub mod known_test_plans;
pub mod list_runs;
pub mod register_known_test_plan;
pub mod run_status;
pub mod test_plan_details;
pub mod trigger;
pub mod whoami;

/// Axum extractor that only admits callers [ServerState::identify_user] resolves to
/// [UserType::Admin], rejecting everyone else with [Error::Unauthorized].
pub struct AdminUser(pub String);

impl FromRequestParts<ServerState> for AdminUser {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        match state.identify_user(&parts.headers).await? {
            UserType::Admin(email) => Ok(Self(email)),
            UserType::User(_) | UserType::Unknown => Err(Error::Unauthorized),
        }
    }
}

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
