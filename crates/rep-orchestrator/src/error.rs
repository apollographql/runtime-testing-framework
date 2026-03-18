use crate::db::Status;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
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
    Io(#[from] io::Error),

    #[error("requested execution status ({requested}) does not follow current status ({current})")]
    InvalidExecutionStatus { current: Status, requested: Status },

    #[error("non-terminal status updates may not include a status code")]
    InvalidExitCode { status: Status, code: u8 },

    #[error("FAILED status updates must have a non-zero exit code")]
    InvalidFailedExitCode,

    #[error("FAILED status updates must include an exit code")]
    MissingExitCode,

    #[error("resolver channel closed")]
    ResolverChannelClosed,

    #[error("{id} is not a known test execution ID")]
    UnknownTestExecution { id: Uuid },

    #[error("{id} is not a known test run ID")]
    UnknownTestRun { id: Uuid },
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let msg = self.to_string();

        let raw = match self {
            Self::MissingExitCode
            | Self::InvalidFailedExitCode
            | Self::InvalidExitCode { .. }
            | Self::InvalidExecutionStatus { .. } => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "BAD_REQUEST", "message": msg })),
            ),

            Self::UnknownTestExecution { id } => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "id": id, "message": msg })),
            ),

            Self::UnknownTestRun { id } => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "id": id, "message": msg })),
            ),

            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "INTERNAL", "message": msg })),
            ),
        };

        raw.into_response()
    }
}
