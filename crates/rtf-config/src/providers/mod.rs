//! Providers are how we expose the rest of the framework to user facing config.
use crate::validation;
use std::{io, path::PathBuf};

pub mod command;
pub mod file;

/// Errors that can be encountered while running file providers
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Execution context for running providers
#[derive(Debug)]
pub struct Context {
    pub(crate) config_dir: PathBuf,
}

impl Context {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}
