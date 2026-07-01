//! The various different config file formats that we support
use crate::{checks, providers};
use std::io;

mod custom_provider;
mod environment;
mod execution;
mod matrix;
mod output_collection;
mod scenario;
mod test_plan;

pub use custom_provider::{CustomProviderDeclaration, CustomProviderDefinition};
pub use environment::{
    DockerComposeEnvironment, EnvironmentConfig, EnvironmentExecution, FileProviderServices,
    ScriptEnvironment,
};
pub use execution::{Execution, Generic};
pub use matrix::Matrix;
pub use output_collection::{OutputCollection, PrometheusQuery};
use rtf_integrations::github;
pub use scenario::{DockerCommand, DockerScenario, ScenarioConfig, ScenarioExecution};
pub use test_plan::{RawTestPlanConfig, Sources, TestPlan, TestPlanConfig};

/// Errors that can be encountered resolving config files
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("One or more file providers failed to run:\n{}", .errs.join("\n"))]
    FailedFileProviders { errs: Vec<String> },

    #[error("One or more custom provider definitions failed to load:\n{}", .errs.join("\n"))]
    FailedCustomProviderDefinitions { errs: Vec<String> },

    #[error("Custom provider declarations can not be specified as part of overrides.")]
    InvalidCustomProviderOverride,

    #[error("The provided variant_names template produced duplicate names: {duplicates:?}")]
    NonUniqueMatrixVariantNames { duplicates: Vec<String> },

    #[error(
        "The provided matrix.variant_names template references unknown matrix variables: {variables:?}"
    )]
    UnknownMatrixVariantTemplateVariables { variables: Vec<String> },

    #[error("The config file being parsed was invalid:\n{0}")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        VariableDefinition,
        checks::Check,
        context::Context,
        providers::file::{FileProvider, NamedFileProvider, RelativeFile, StableSource},
        templating::{ErrorKind, Field, Scalar, Template, TemplateContext},
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

    /// Create a HashMap of variables from string names (each name maps to itself as a Scalar::String)
    pub(crate) fn template_context(variable_names: &[&str]) -> TemplateContext {
        TemplateContext::new_stubbed(
            variable_names
                .iter()
                .map(|&name| (name.to_string(), Scalar::String(name.to_string())))
                .collect(),
        )
    }

    /// Create VariableDefinitions from string names with default description
    pub(crate) fn variable_definitions(variable_names: &[&str]) -> Vec<VariableDefinition> {
        variable_names
            .iter()
            .map(|&name| VariableDefinition {
                name: name.to_string(),
                description: "description".to_string(),
                default: None,
                allowed_values: None,
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
        ctx: TemplateContext,
        expected_err_messages: Vec<String>,
        expected_err_paths: Vec<String>,
    ) {
        let res = t.try_template(&mut Vec::new(), &StableSource::TestPlan, &ctx);
        assert!(res.is_err(), "expected templating to fail, got {res:?}");

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::UnknownVariable)),
            "expected all errors to be UnknownVariable, got {:?}",
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
        ctx: &Context,
        expected_err_kinds: &[checks::ErrorKind],
    ) {
        let res = c.try_check(&mut Vec::new(), ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err();
        let err_kinds: Vec<checks::ErrorKind> = err.iter().map(|e| e.kind).collect();
        assert_eq!(
            err_kinds, expected_err_kinds,
            "check the error kind is correct"
        );
    }
}
