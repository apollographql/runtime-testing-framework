//! The various different config file formats that we support
use crate::{ValueDefinition, checks, providers, templating::Scalar};
use std::{collections::HashMap, io};

mod environment;
mod matrix;
mod scenario;
mod test_plan;

pub use environment::EnvironmentConfig;
pub use matrix::Matrix;
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

    #[error("the provided variant_names template produced duplicate names: {duplicates:?}")]
    NonUniqueMatrixVariantNames { duplicates: Vec<String> },

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
    use crate::{
        checks::Check,
        context::Context,
        providers::file::{FileProvider, NamedFileProvider, RelativeFile, Source},
        templating::{ErrorKind, Field, Template},
    };

    // Test Helpers
    // These are used across the formats tests, not necessarily in the tests below

    /// Return a pending field
    pub(crate) fn p(name: &str) -> Field<String> {
        Field::Pending(name.to_string())
    }

    /// Return a resolved field
    pub(crate) fn r(name: &str) -> Field<String> {
        Field::Resolved(name.to_string())
    }

    /// Return a NamedFileProvider with a field
    pub(crate) fn named_file_provider_with_field(
        name: &str,
        f: Field<String>,
    ) -> NamedFileProvider {
        NamedFileProvider {
            name: name.to_string(),
            env_var: name.to_ascii_uppercase(),
            provider: FileProvider::RelativePath(RelativeFile { path: f, src: None }),
        }
    }

    /// Return named file providers with fields
    pub(crate) fn named_file_providers_with_fields(
        fields: &[Field<String>],
    ) -> Vec<NamedFileProvider> {
        fields
            .iter()
            .enumerate()
            .map(|(i, f)| named_file_provider_with_field(&format!("file{}", i), f.clone()))
            .collect()
    }

    /// Create NamedFileProviders with pending fields from string names
    pub(crate) fn templatable_file_providers(field_names: &[&str]) -> Vec<NamedFileProvider> {
        field_names
            .iter()
            .map(|name| named_file_provider_with_field(name, p(name)))
            .collect()
    }

    /// Create a HashMap of values from string names (each name maps to itself as a Scalar::String)
    pub(crate) fn value_map(value_names: &[&str]) -> HashMap<String, Scalar> {
        value_names
            .iter()
            .map(|&name| (name.to_string(), Scalar::String(name.to_string())))
            .collect()
    }

    /// Create ValueDefinitions from string names with default description
    pub(crate) fn value_definitions(value_names: &[&str]) -> Vec<ValueDefinition> {
        value_names
            .iter()
            .map(|&name| ValueDefinition {
                name: name.to_string(),
                description: "description".to_string(),
                default: None,
            })
            .collect()
    }

    /// Generate expected error details for fields
    pub(crate) fn expected_error_details(
        field_names: &[&str],
        path_prefix: &str,
    ) -> (Vec<String>, Vec<String>) {
        let mut expected_messages: Vec<String> =
            field_names.iter().map(|name| name.to_string()).collect();
        expected_messages.sort();

        let mut expected_paths: Vec<String> = field_names
            .iter()
            .map(|name| {
                format!(
                    "{}.file_providers.{}.path",
                    path_prefix,
                    name.to_ascii_uppercase()
                )
            })
            .collect();
        expected_paths.sort();

        (expected_messages, expected_paths)
    }

    /// Assert template errors
    pub(crate) fn assert_template_errors(
        t: &mut impl Template,
        values: HashMap<String, Scalar>,
        expected_err_messages: Vec<String>,
        expected_err_paths: Vec<String>,
    ) {
        let res = t.try_template(&mut Vec::new(), &values);
        assert!(res.is_err(), "expected templating to fail, got {res:?}");

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::UnknownValue)),
            "expected all errors to be UnknownValue, got {:?}",
            errors
        );

        let mut messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
        messages.sort();
        assert_eq!(
            messages, expected_err_messages,
            "we are testing we get all expected error messages"
        );

        let mut paths: Vec<String> = errors.iter().map(|e| e.path.clone()).collect();
        paths.sort();
        assert_eq!(
            paths, expected_err_paths,
            "we are testing we get all expected error paths"
        );
    }

    /// Assert check errors
    pub(crate) fn assert_check_errors(
        c: impl Check,
        src: &Source,
        ctx: &Context,
        expected_err_kinds: &[checks::ErrorKind],
    ) {
        let res = c.try_check(&mut Vec::new(), src, ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err();
        let err_kinds: Vec<checks::ErrorKind> = err.iter().map(|e| e.kind).collect();
        assert_eq!(
            err_kinds, expected_err_kinds,
            "check the error kind is correct"
        );
    }

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
