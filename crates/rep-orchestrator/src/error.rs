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

    #[error("resolver channel closed")]
    ResolverChannelClosed,

    #[error("{id} is not a known test execution ID")]
    UnknownTestExecution { id: Uuid },

    #[error("{id} is not a known test run ID")]
    UnknownTestRun { id: Uuid },
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let raw = match self {
            Self::UnknownTestExecution { id } => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "id": id })),
            ),

            Self::UnknownTestRun { id } => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "NOT_FOUND", "id": id })),
            ),

            err => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "INTERNAL", "message": err.to_string() })),
            ),
        };

        raw.into_response()
    }
}
