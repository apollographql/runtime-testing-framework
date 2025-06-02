//! Providers are how we expose the rest of the framework to user facing config.
use std::io;

pub mod command;
pub mod file;

/// Errors that can be encountered while running file providers
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
