use crate::{
    db::{self, ClusterId},
    resolver::ResolverError,
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use rtf_config::formats;
use rtf_integrations::github;
use rtf_orchestrator_shared::{payload::PrepareError, test_plan_details::EnvironmentSummaryError};
use serde::Serialize;
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

    #[error("{reason} on cluster {cluster} for this user")]
    RateLimited {
        cluster: ClusterId,
        reason: RateLimitReason,
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

    #[error("{cluster} is not a configured workload cluster")]
    UnknownWorkloadCluster { cluster: String },
}

/// Why a trigger request was rejected for exceeding a per-user rate limit, and on which cluster
/// the test plan would have run.
#[derive(thiserror::Error, Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RateLimitReason {
    #[error("{current}/{max} concurrent runs")]
    ConcurrentRuns { current: u64, max: u64 },

    #[error("{current}/{max} queued runs")]
    QueuedRuns { current: u64, max: u64 },

    #[error("{current}/{max} queued executions")]
    QueuedExecutions { current: u64, max: u64 },

    #[error("{current}/{max} runs in the last hour")]
    RunsPerHour { current: u64, max: u64 },
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let msg = self.to_string();

        let raw = match self {
            Self::FileUploadAlreadyRequested
            | Self::InvalidFileProviderUsage { .. }
            | Self::UnknownWorkloadCluster { .. }
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

            Self::RateLimited { cluster, reason } => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(
                    json!({ "error": "TOO_MANY_REQUESTS", "message": msg, "cluster": cluster, "reason": reason }),
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
