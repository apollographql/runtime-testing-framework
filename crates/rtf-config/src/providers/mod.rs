//! Providers are how we expose the rest of the framework to user facing config.
use rtf_core::graphos::supergraph::FetchError;
use serde::Serialize;
use std::io;

use crate::providers::{command::CommandProvider, file::FileProvider};

pub mod command;
pub mod file;

/// Errors that can be encountered while running file providers
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Fetch(#[from] FetchError),

    #[error(transparent)]
    Github(#[from] rtf_core::github::Error),

    #[error(transparent)]
    GraphOS(#[from] rtf_core::graphos::Error),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    #[error("Request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Missing provider output for {name}")]
    MissingProviderOutput { name: String },

    #[error("Error decoding bytes to utf8")]
    Utf8DecodingError,

    #[error("Unknown router version: {0}")]
    UnknownRouterVersion(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Wrapper enum for supporting caching of provider output
#[derive(Debug, Serialize)]
pub enum Provider<'a> {
    File {
        fp: &'a FileProvider,
    },
    Command {
        name: &'a str,
        cmd: &'a CommandProvider,
    },
}
