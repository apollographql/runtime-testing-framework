//! The various different config file formats that we support
use crate::{providers, validation};
use std::{collections::HashMap, io};

mod environment;
mod test_plan;

pub use environment::EnvironmentConfig;
pub use test_plan::{BaseTestPlanConfig, RawBaseTestPlanConfig};

/// Errors that can be encountered resolving config files
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("one or more file providers failed to run:\n{}", .errs.join("\n"))]
    FailedFileProviders { errs: Vec<String> },

    #[error("the config file being parsed was invalid:\n{0}")]
    Validation(#[from] validation::Errors),

    // wrapped errors
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Provider(#[from] providers::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
