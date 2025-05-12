//! The various different config file formats that we support
use crate::providers;
use std::io;

mod environment;

pub use environment::{EnvironmentConfig, RawEnvironmentConfig};

/// Errors that can be encountered while running file providers
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("one or more file providers failed to run:\n{}", .errs.join("\n"))]
    FailedFileProviders { errs: Vec<String> },

    #[error("one or more file providers were invalid:\n{}", .errs.join("\n"))]
    InvalidFileProviders { errs: Vec<String> },

    // wrapped errors
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Provider(#[from] providers::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
