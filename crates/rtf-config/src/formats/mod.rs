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
/// file. This is also where defaults defined in value definitions are applied, being overwritten
/// by any explicitly provided values coming from `all_values`.
///
/// # Constructing the definitions argument
///
/// The trait bound here is to support both direct calls to `Vec<ValueDefinition>.iter()` and calls
/// to [Iterator::chain] to joining together multiple vecs of ValueDefinitions:
///
/// ```ignore
/// // from EnvironmentConfig: both of these will work
/// let definitions = self.values.iter();
/// let definitions = self.values.iter().chain(self.setup.provides.iter());
/// ```
pub(crate) fn values_for_config_file<'a>(
    all_values: &HashMap<String, Scalar>,
    definitions: impl Iterator<Item = &'a ValueDefinition> + Clone,
) -> HashMap<String, Scalar> {
    let mut values: HashMap<String, Scalar> = definitions
        .clone()
        .flat_map(|vd| vd.default.clone().map(|v| (vd.name.clone(), v)))
        .collect();

    values.extend(
        all_values
            .iter()
            .filter(|(k, _)| definitions.clone().any(|val| &val.name == *k))
            .map(|(k, v)| (k.clone(), v.clone())),
    );

    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_defaults_are_used_correctly() {
        let all_values: HashMap<String, Scalar> = [
            ("a".into(), 1.into()),
            ("b".into(), "foo".into()),
            ("c".into(), true.into()),
        ]
        .into_iter()
        .collect();

        let definitions = [
            ValueDefinition {
                name: "a".into(),
                description: String::new(),
                default: Some(2.into()),
            },
            ValueDefinition {
                name: "b".into(),
                description: String::new(),
                default: None,
            },
            ValueDefinition {
                name: "d".into(),
                description: String::new(),
                default: Some("bar".into()),
            },
        ];

        let vals = values_for_config_file(&all_values, definitions.iter());

        // a has an explicit value so it overrides the default
        // b has an explicit value and no default
        // c is not in the definitions so it is filtered out
        // d has no explicit value so we take the default
        let expected: HashMap<String, Scalar> = [
            ("a".into(), 1.into()),
            ("b".into(), "foo".into()),
            ("d".into(), "bar".into()),
        ]
        .into_iter()
        .collect();

        assert_eq!(vals, expected);
    }
}
