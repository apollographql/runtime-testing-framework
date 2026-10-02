use crate::{
    db::{self, PoolId},
    rate_limit::RateLimitError,
    resolver::ResolverError,
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use rtf_config::formats;
use rtf_integrations::github;
use rtf_orchestrator_shared::{
    payload::PrepareError, test_plan_details::EnvironmentSummaryError, workload_config,
};
use serde_json::json;
use std::io;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] crate::db::Error),

    #[error(transparent)]
    Gcs(#[from] crate::gcs::Error),

    #[error(transparent)]
    GitHub(#[from] github::Error),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Resolve(#[from] ResolverError),

    #[error(transparent)]
    RtfConfig(#[from] formats::Error),

    #[error(transparent)]
    Prepare(#[from] PrepareError),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    #[error("file upload has already been requested for this execution")]
    FileUploadAlreadyRequested,

    #[error("no file upload available for this execution")]
    FileUploadNotAvailable,

    #[error("files not ready for download")]
    FileUploadNotReady,

    #[error("insufficient queue capacity")]
    InsufficientCapacity,

    #[error("manual file provider volume mounts are not supported. Invalid services: {services:?}")]
    InvalidFileProviderUsage { services: Vec<String> },

    #[error("{reason} in pool {pool} for this user")]
    RateLimited {
        pool: PoolId,
        reason: RateLimitError,
    },

    #[error("resolver channel closed")]
    ResolverChannelClosed,

    #[error("{id} is not a known test execution ID")]
    UnknownTestExecution { id: Uuid },

    #[error("not authorized for this execution")]
    Unauthorized,

    #[error("{id} is not a known test run ID")]
    UnknownTestRun { id: Uuid },

    #[error("{identifier} is not a registered test plan")]
    UnknownTestPlan { identifier: String },

    #[error("{pool} is not a configured workload cluster pool")]
    UnknownWorkloadPool { pool: String },

    #[error("this test plan requires a dedicated cluster but pool {pool} does not support them")]
    DedicatedClusterNotSupported { pool: String },

    #[error("invalid workload config: {0}")]
    InvalidWorkloadConfig(#[from] workload_config::Error),

    #[error("node labels can only be set for a test plan that requires a dedicated cluster")]
    NodeLabelsRequireDedicatedCluster,

    #[error("node labels can only be set for a test plan that is pinned to pool")]
    NodeLabelsRequirePinnedPool,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let msg = self.to_string();

        let raw = match self {
            Self::FileUploadAlreadyRequested
            | Self::InvalidFileProviderUsage { .. }
            | Self::UnknownWorkloadPool { .. }
            | Self::DedicatedClusterNotSupported { .. }
            | Self::InvalidWorkloadConfig(_)
            | Self::NodeLabelsRequireDedicatedCluster
            | Self::NodeLabelsRequirePinnedPool
            | Self::Prepare(_)
            | Self::Db(db::Error::MissingExitCode)
            | Self::Db(db::Error::InvalidFailedExitCode)
            | Self::Db(db::Error::InvalidExitCode { .. })
            | Self::Db(db::Error::InvalidExecutionStatus { .. }) => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "BAD_REQUEST", "message": msg })),
            ),

            Self::FileUploadNotAvailable => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "message": msg })),
            ),

            Self::FileUploadNotReady | Self::Db(db::Error::KnownTestPlanAlreadyExists) => (
                StatusCode::CONFLICT,
                Json(json!({ "error": "CONFLICT", "message": msg })),
            ),

            Self::InsufficientCapacity => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "SERVICE_UNAVAILABLE", "message": msg })),
            ),

            Self::RateLimited { pool, reason } => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(
                    json!({ "error": "TOO_MANY_REQUESTS", "message": msg, "pool": pool,  "reason": reason }),
                ),
            ),

            Self::Unauthorized => (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "FORBIDDEN", "message": msg })),
            ),

            Self::UnknownTestExecution { id } => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "id": id, "message": msg })),
            ),

            Self::UnknownTestRun { id } => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "id": id, "message": msg })),
            ),

            Self::UnknownTestPlan { identifier } => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "identifier": identifier, "message": msg })),
            ),

            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "INTERNAL", "message": msg })),
            ),
        };

        raw.into_response()
    }
}

impl From<EnvironmentSummaryError> for Error {
    fn from(err: EnvironmentSummaryError) -> Self {
        match err {
            EnvironmentSummaryError::Formats(e) => e.into(),
            EnvironmentSummaryError::Inlining(e) => ResolverError::Inlining(e).into(),
            EnvironmentSummaryError::Templating(e) => ResolverError::TemplatingCheck(e).into(),
        }
    }
}
