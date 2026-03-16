use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] crate::db::Error),

    #[error(transparent)]
    Io(#[from] io::Error),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {self}"),
        )
            .into_response()
    }
}
