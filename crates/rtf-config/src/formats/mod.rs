//! The various different config file formats that we support
use crate::{ValueDefinition, checks, providers, templating::Scalar};
use std::{collections::HashMap, io};

mod environment;
mod scenario;
mod test_plan;

pub use environment::EnvironmentConfig;
use rtf_core::github;
pub use scenario::ScenarioConfig;
pub use test_plan::{RawTestPlanConfig, TestPlanConfig};

/// Errors that can be encountered resolving config files
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("one or more file providers failed to run:\n{}", .errs.join("\n"))]
    FailedFileProviders { errs: Vec<String> },

    #[error("missing required output fields from environment setup: {missing:?}")]
    InvalidSetupOutput { missing: Vec<String> },

    #[error("the config file being parsed was invalid:\n{0}")]
    Validation(#[from] checks::Errors),

    // wrapped errors
    #[error(transparent)]
    GitHub(#[from] github::Error),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Provider(#[from] providers::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Helper for filtering allowed templating values based on [ValueDefinition]s present in a config
/// file.
///
/// # Constructing the definitions argument
///
/// The trait bound here is to support both direct calls to `Vec<ValueDefinition>.iter()` and calls
/// to [Iterator::chain] to joing together multiple vecs of ValueDefintions:
///
/// ```ignore
/// // from EnvironmentConfig: both of these will work
/// let definitions = self.values.iter();
/// let definitions = self.values.iter().chain(self.setup.provides.iter());
/// ```
pub(crate) fn filter_values<'a>(
    all_values: &HashMap<String, Scalar>,
    definitions: impl Iterator<Item = &'a ValueDefinition> + Clone,
) -> HashMap<String, Scalar> {
    all_values
        .iter()
        .filter(|(k, _)| definitions.clone().any(|val| &val.name == *k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
