//! Parsing of the environment config file format
use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray, duplicate_keys},
    context::ResolutionContext,
    enum_impl_check, enum_impl_check_array_duplicates, enum_impl_run_environment,
    enum_impl_run_providers,
    formats::{CustomProviderDeclaration, OutputCollection, Result},
    inlining::{self, InlineMode, InlinedProvider},
    providers::{self, file::StableSource},
    run::{Provider, RunEnvironment, RunProviders, ValidateEnvironment},
    templating::{self, FileType, Template, TemplateContext},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    pin::Pin,
};

mod docker_compose;
mod k8s;
mod manifest;
mod null;
mod script;

pub use docker_compose::{
    ComposeResources, DockerComposeEnvironment, EnvironmentService, FileProviderServices,
    PullPolicyServices, ServiceReplicas,
};
pub use k8s::{K8sEnvironment, K8sResources};
pub use manifest::{ManifestEnvironment, NamedManifestFiles};
pub use null::NullEnvironment;
pub use script::ScriptEnvironment;

/// # Environment Config
///
/// Configuration for preparing and cleaning up the test environment as part of a test plan.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct EnvironmentConfig<T: ValidateEnvironment> {
    /// The name of this environment configuration
    pub name: String,
    /// A brief description of how this environment setup works
    pub description: String,
    /// Definitions for the required variables for templating this environment
    #[serde(default, alias = "values")]
    // This alias is for backwards compatibility with the original name
    pub variable_definitions: Vec<VariableDefinition>,
    /// Custom provider declarations to load for this environment
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderDeclaration>,
    #[serde(flatten)]
    pub execution: T,
}

impl EnvironmentConfig<EnvironmentExecution> {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    pub fn output_collection(&self) -> Option<&OutputCollection> {
        match &self.execution {
            EnvironmentExecution::DockerCompose(ex) => Some(&ex.output_collection),
            EnvironmentExecution::Script(_) => None,
            EnvironmentExecution::Null(_) => None,
        }
    }

    /// Returns `true` if this environment actually has output configured to collect
    pub fn output_collection_defined(&self) -> bool {
        self.output_collection()
            .is_some_and(OutputCollection::is_defined)
    }

    /// Create an empty [EnvironmentConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> EnvironmentConfig<EnvironmentExecution> {
        use crate::providers::command::CommandSection;

        EnvironmentConfig {
            name: Default::default(),
            description: Default::default(),
            variable_definitions: Vec::new(),
            custom_providers: Default::default(),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection::empty(),
                teardown: CommandSection::empty(),
            }),
        }
    }
}

impl<T: RunEnvironment> EnvironmentConfig<T> {
    pub async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.execution.execute_setup(name, out_dir, ctx).await
    }

    pub async fn execute_teardown(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.execution.execute_teardown(name, out_dir, ctx).await
    }
}

impl<T: ValidateEnvironment> EnvironmentConfig<T> {
    pub async fn inline(
        &mut self,
        mode: &InlineMode,
        ctx: &impl ResolutionContext,
        cache: &mut HashMap<u64, InlinedProvider>,
    ) -> inlining::Result<()> {
        self.execution.inline(mode, ctx, cache).await
    }
}

impl<T: ValidateEnvironment> Template for EnvironmentConfig<T> {
    fn required_variables(&self) -> Vec<String> {
        self.execution.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut allowed_variables = allowed_variables.clone();
        allowed_variables.extend(self.variable_definitions.iter().map(|vd| &vd.name));

        let ctx = ctx.for_config_file(
            file_source,
            Some(FileType::Environment),
            self.variable_definitions.iter(),
        );

        self.execution
            .validate_context(path, &allowed_variables, file_source, &ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let ctx = ctx.for_config_file(
            source,
            Some(FileType::Environment),
            self.variable_definitions.iter(),
        );

        // We call try_template here instead of try_template_nested to avoid appending
        // an unnecessary entry to the path
        self.execution.try_template(path, source, &ctx)
    }
}

impl<T: ValidateEnvironment> Check for EnvironmentConfig<T> {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        // Check that hard coded variables are unique
        let all_variables = self.variable_definitions.iter();
        let duplicates = duplicate_keys(all_variables, |v| &v.name);
        if !duplicates.is_empty() {
            errs.push(
                checks::ErrorKind::DuplicateVariableNames,
                duplicates.join("\n"),
                path,
            );
        }

        // Check that each command is valid in isolation
        // We call try_check here instead of try_check_nested to avoid appending
        // an unnecessary entry to the path
        errs.append(self.execution.try_check(path, ctx));

        errs.into_result(())
    }
}

impl<T: ValidateEnvironment> CheckArrayDuplicates for EnvironmentConfig<T> {
    const BASE_PATH: &str = "environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        let mut arrays = vec![(
            "variables",
            DedupArray::VariableDef(&mut self.variable_definitions),
        )];
        arrays.extend(self.execution.deduplicated_arrays());

        arrays
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(
    untagged,
    expecting = "expected null environment (skip: true), docker-compose environment (with compose_files) or script environment (with setup/teardown)"
)]
#[allow(clippy::large_enum_variant)] // We only ever allocate one of these, not multiples, so the difference in variant size should not be an issue
pub enum EnvironmentExecution {
    // Null needs to be the first variant in this enum to ensure that any time `skip: true` is set,
    // we resolve to a NullEnvironment.
    Null(NullEnvironment),
    DockerCompose(DockerComposeEnvironment),
    Script(ScriptEnvironment),
}

impl ValidateEnvironment for EnvironmentExecution {}

enum_impl_check!(EnvironmentExecution => Null, DockerCompose, Script);
enum_impl_run_providers!(EnvironmentExecution => Null, DockerCompose, Script);
enum_impl_run_environment!(EnvironmentExecution => Null, DockerCompose, Script);
enum_impl_check_array_duplicates!(EnvironmentExecution, "environment_execution" => Null, DockerCompose, Script);

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(
    untagged,
    expecting = "expected null environment (skip: true), docker-compose environment (with compose_files), script environment (with setup/teardown), or kubernetes environment (with resources)"
)]
#[allow(clippy::large_enum_variant)] // We only ever allocate one of these, not multiples, so the difference in variant size should not be an issue
pub enum EnvironmentPrepare {
    // Null needs to be the first variant in this enum to ensure that any time `skip: true` is set,
    // we resolve to a NullEnvironment.
    Null(NullEnvironment),
    DockerCompose(DockerComposeEnvironment),
    Script(ScriptEnvironment),
    K8s(K8sEnvironment),
}

impl ValidateEnvironment for EnvironmentPrepare {}

enum_impl_check!(EnvironmentPrepare => Null, DockerCompose, Script, K8s);
enum_impl_run_providers!(EnvironmentPrepare => Null, DockerCompose, Script, K8s);
enum_impl_check_array_duplicates!(EnvironmentPrepare, "environment_prepare" => Null, DockerCompose, Script, K8s);

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use crate::{
        context::Context,
        formats::{
            OutputCollection,
            tests::{
                named_file_providers_with_fields, templatable_file_providers, variable_definitions,
            },
        },
        providers::{
            command::CommandSection,
            file::{
                InlineFile,
                manifest::{ManifestFileProvider, NamedManifestFileProvider},
            },
        },
        run::Provider,
        templating::Field,
    };
    use assert_fs::{TempDir, prelude::*};
    use std::path::PathBuf;

    /// Create an EnvironmentConfig for testing Template trait methods (has_pending_fields, required_variables)
    pub(crate) fn environment_with_fields(
        setup_fields: &[Field<String>],
        teardown_fields: &[Field<String>],
        custom_providers: &[CustomProviderDeclaration],
    ) -> EnvironmentConfig<EnvironmentExecution> {
        EnvironmentConfig {
            custom_providers: custom_providers.to_vec(),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection {
                    file_providers: named_file_providers_with_fields(setup_fields),
                    ..CommandSection::empty()
                },
                teardown: CommandSection {
                    file_providers: named_file_providers_with_fields(teardown_fields),
                    ..CommandSection::empty()
                },
            }),
            ..EnvironmentConfig::empty()
        }
    }

    /// Create a test EnvironmentConfig for template tests
    pub(crate) fn templatable_environment(
        variable_names: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        custom_providers: &[CustomProviderDeclaration],
    ) -> EnvironmentConfig<EnvironmentExecution> {
        EnvironmentConfig {
            custom_providers: custom_providers.to_vec(),
            variable_definitions: variable_definitions(variable_names),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection {
                    file_providers: templatable_file_providers(setup_fields),
                    ..CommandSection::empty()
                },
                teardown: CommandSection {
                    file_providers: templatable_file_providers(teardown_fields),
                    ..CommandSection::empty()
                },
            }),
            ..EnvironmentConfig::empty()
        }
    }

    /// Create a named compose file with inline content unique to the name.
    /// Using the name in the content ensures each compose file has a unique provider
    /// identity (since providers are keyed by their serialized content).
    pub(crate) fn named_compose_file(name: &str) -> NamedManifestFileProvider {
        NamedManifestFileProvider {
            name: name.to_string(),
            provider: ManifestFileProvider::Inline(InlineFile {
                content: format!("# {name}\nservices: {{}}"),
            }),
        }
    }

    /// Create a simple DockerComposeEnvironment for command-building tests
    pub(crate) fn docker_compose_env(
        project_name: Option<&str>,
        compose_file_names: &[&str],
    ) -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            resources: docker_compose::ComposeResources {
                project_name: project_name.map(String::from),
                compose_files: compose_file_names
                    .iter()
                    .map(|name| named_compose_file(name))
                    .collect(),
            },
            file_providers: Vec::new(),
            env_vars: HashMap::new(),
            output_collection: OutputCollection {
                prometheus: Vec::new(),
            },
        }
    }

    /// Create temp files and store their paths in context, returning the paths for use in assertions.
    /// The returned TempDir must be kept alive for the duration of the test to prevent cleanup.
    pub(crate) fn register_compose_paths(
        ctx: &mut Context,
        env: &DockerComposeEnvironment,
    ) -> (TempDir, Vec<String>) {
        let temp_dir = TempDir::new().unwrap();
        let paths = env
            .resources
            .compose_files
            .iter()
            .map(|ncfp| {
                let file = temp_dir.child(&ncfp.name);
                file.write_str(&format!("# {}\nservices: {{}}", ncfp.name))
                    .unwrap();
                let path = file.path().to_string_lossy().to_string();
                ctx.store_provider_output_path(
                    Provider::ComposeFile { fp: &ncfp.provider },
                    PathBuf::from(&path),
                );
                path
            })
            .collect();
        (temp_dir, paths)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        checks::ErrorKind,
        context::Context,
        formats::{
            OutputCollection,
            environment::test_helpers::{docker_compose_env, environment_with_fields},
            output_collection::PrometheusQuery,
            tests::{assert_check_errors, p, r, variable_definitions},
        },
        providers::{
            self,
            command::{CommandSection, test_helpers::cmd_with_required_file},
            file::{RawSource, SourceDir},
            test_helpers::create_temp_dir_with_file,
        },
        templating::Field,
    };
    use assert_fs::{
        fixture::PathChild,
        prelude::{FileWriteStr, PathCreateDir},
    };
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::{assert_matches, collections::HashMap};

    #[test_case(&[p("setup1"), p("setup2")], &[p("teardown1"), p("teardown2")], &["setup1", "setup2", "teardown1", "teardown2"]; "both setup and both teardown pending requires variables")]
    #[test_case(&[p("setup1"), p("setup2")], &[p("teardown1"), r("teardown2")], &["setup1", "setup2", "teardown1"]; "both setup and single teardown pending requires variables")]
    #[test_case(&[p("setup1"), p("setup2")], &[r("teardown1"), r("teardown2")], &["setup1", "setup2"]; "both setup and no teardown pending requires variables")]
    #[test_case(&[p("setup1"), r("setup2")], &[p("teardown1"), p("teardown2")], &["setup1", "teardown1", "teardown2"]; "single setup and both teardown pending requires variables")]
    #[test_case(&[p("setup1"), r("setup2")], &[p("teardown1"), r("teardown2")], &["setup1", "teardown1"]; "single setup and single teardown pending requires variables")]
    #[test_case(&[p("setup1"), r("setup2")], &[r("teardown1"), r("teardown2")], &["setup1"]; "single setup and no teardown pending requires variables")]
    #[test_case(&[r("setup1"), r("setup2")], &[p("teardown1"), p("teardown2")], &["teardown1", "teardown2"]; "no setup and both teardown pending requires variables")]
    #[test_case(&[r("setup1"), r("setup2")], &[p("teardown1"), r("teardown2")], &["teardown1"]; "no setup and single teardown pending requires variables")]
    #[test_case(&[r("setup1"), r("setup2")], &[r("teardown1"), r("teardown2")], &[]; "no setup and no teardown pending requires no variables")]
    #[test]
    fn required_variables(
        setup_fields: &[Field<String>],
        teardown_fields: &[Field<String>],
        expected: &[&str],
    ) {
        let environment = environment_with_fields(setup_fields, teardown_fields, &[]);

        let res = environment.required_variables();
        assert_eq!(
            res, expected,
            "tests that required_variables has expected value"
        )
    }

    #[test_case(
        cmd_with_required_file(),
        CommandSection::empty(),
        &[],
        &[ErrorKind::RequiredFileMissing];
        "setup only"
    )]
    #[test_case(
        CommandSection::empty(),
        cmd_with_required_file(),
        &[],
        &[ErrorKind::RequiredFileMissing];
        "teardown only"
    )]
    #[test_case(
        cmd_with_required_file(),
        cmd_with_required_file(),
        &["foo", "foo"],
        &[ErrorKind::DuplicateVariableNames, ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "environment duplicate variables"
    )]
    #[test]
    fn try_check_errors(
        setup_command: CommandSection,
        teardown_command: CommandSection,
        variables: &[&str],
        expected_err_kinds: &[ErrorKind],
    ) {
        let environment = EnvironmentConfig {
            variable_definitions: variable_definitions(variables),
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: setup_command,
                teardown: teardown_command,
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        assert_check_errors(environment, &ctx, expected_err_kinds);
    }

    #[test]
    fn try_check_errors_invalid_prometheus_query() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                output_collection: OutputCollection {
                    prometheus: vec![PrometheusQuery {
                        name: "name".to_string(),
                        step: "15m".to_string(),
                        query: "not a valid query".to_string(),
                    }],
                },
                ..docker_compose_env(Some("project"), &["file"])
            }),

            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        assert_check_errors(environment, &ctx, &[checks::ErrorKind::InvalidPromQl]);
    }

    #[test]
    fn output_collection_defined_false_for_docker_compose_environment_without_prometheus_queries() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::DockerCompose(docker_compose_env(
                Some("project"),
                &["file"],
            )),
            ..EnvironmentConfig::empty()
        };

        assert!(!environment.output_collection_defined());
    }

    #[test]
    fn output_collection_defined_true_for_docker_compose_environment_with_prometheus_queries() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                output_collection: OutputCollection {
                    prometheus: vec![PrometheusQuery {
                        name: "name".to_string(),
                        step: "15m".to_string(),
                        query: "sum(rate(metric[1m]))".to_string(),
                    }],
                },
                ..docker_compose_env(Some("project"), &["file"])
            }),
            ..EnvironmentConfig::empty()
        };

        assert!(environment.output_collection_defined());
    }

    #[test]
    fn output_collection_defined_false_for_script_environment() {
        let environment = EnvironmentConfig::empty();

        assert!(!environment.output_collection_defined());
    }

    #[test]
    fn output_collection_defined_false_for_null_environment() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::Null(NullEnvironment { skip: true }),
            ..EnvironmentConfig::empty()
        };

        assert!(!environment.output_collection_defined());
    }

    #[test]
    fn skip_true_alongside_valid_non_null_environmment_parses_as_null_environment() {
        let yaml = indoc!(
            r#"
            name: test
            description: test
            compose_files:
              - name: compose.yaml
                kind: inline
                content: "services: {}"
            skip: true"#
        );

        let config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(yaml).expect("environment config to parse");

        assert_matches!(config.execution, EnvironmentExecution::Null(_));
    }

    #[test]
    fn skip_false_alongside_valid_non_null_environmment_parses_as_that_environment() {
        let yaml = indoc!(
            r#"
            name: test
            description: test
            compose_files:
              - name: compose.yaml
                kind: inline
                content: "services: {}"
            skip: false"#
        );

        let config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(yaml).expect("environment config to parse");

        assert_matches!(config.execution, EnvironmentExecution::DockerCompose(_));
    }

    #[tokio::test]
    async fn custom_provider_try_load_all_unknown_file_errors() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: HashMap::from([("missing_provider".into(), "missing.yaml".into())]),
        };

        let res = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await;

        assert!(res.is_err());
        let errors = res.unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "missing_provider");
        assert!(matches!(errors[0].1, providers::Error::Io(_)));
    }

    const CUSTOM_PROVIDER_WITH_NESTED: &str = indoc!(
        r#"
        name: invalid provider
        description: A custom provider with nested custom_providers
        variable_definitions: []
        command:
          name: script.sh
          kind: relative_path
          path: ./script.sh
        custom_providers:
          - kind: local
            relative_path: ./nested
            using:
              nested_provider: nested.yaml
        "#
    );

    #[tokio::test]
    async fn custom_provider_try_load_all_nested_custom_provider_errors() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        providers_dir
            .child("invalid.yaml")
            .write_str(CUSTOM_PROVIDER_WITH_NESTED)
            .unwrap();

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: HashMap::from([("invalid_provider".into(), "invalid.yaml".into())]),
        };

        let result = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await;

        assert!(result.is_err());
        let errors = result.unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "invalid_provider");
        assert!(matches!(
            errors[0].1,
            providers::Error::NestedCustomProvider
        ));
    }

    #[tokio::test]
    async fn custom_provider_try_load_all_multiple_errors() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        let valid_provider = indoc!(
            r#"
            name: valid provider
            description: A valid custom provider
            variable_definitions: []
            command:
              name: script.sh
              kind: relative_path
              path: ./script.sh
            "#
        );

        providers_dir
            .child("valid.yaml")
            .write_str(valid_provider)
            .unwrap();

        providers_dir
            .child("invalid.yaml")
            .write_str(CUSTOM_PROVIDER_WITH_NESTED)
            .unwrap();

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: HashMap::from([
                ("valid_provider".into(), "valid.yaml".into()),
                ("invalid_provider".into(), "invalid.yaml".into()),
                ("missing_provider".into(), "missing.yaml".into()),
            ]),
        };

        let result = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await;

        assert!(result.is_err());
        let errors = result.unwrap_err();

        assert_eq!(errors.len(), 2);

        // Errors should be sorted alphabetically by key in the "using" map
        assert_eq!(errors[0].0, "invalid_provider");
        assert_eq!(errors[1].0, "missing_provider");

        assert!(matches!(
            errors[0].1,
            providers::Error::NestedCustomProvider
        ));
        assert!(matches!(errors[1].1, providers::Error::Io(_)));
    }

    #[tokio::test]
    async fn custom_providers_integration() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        let simple_provider = indoc!(
            r#"
        name: simple provider
        description: A simple custom provider for integration testing
        variable_definitions:
          - name: test_var
            description: a test variable
        command:
          name: provider.sh
          kind: relative_path
          path: ./provider.sh
        "#
        );

        providers_dir
            .child("provider1.yaml")
            .write_str(simple_provider)
            .unwrap();
        providers_dir
            .child("provider2.yaml")
            .write_str(simple_provider)
            .unwrap();

        let env_config_yaml = indoc!(
            r#"
            name: test environment
            description: Environment with custom providers
            variable_definitions: []
            custom_providers:
              - kind: local
                relative_path: providers
                using:
                  provider1: provider1.yaml
                  provider2: provider2.yaml
            setup:
              command:
                name: setup.sh
                kind: relative_path
                path: ./setup.sh
            teardown:
              command:
                name: teardown.sh
                kind: relative_path
                path: ./teardown.sh
            "#
        );

        let env_config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(env_config_yaml).expect("environment config to parse");

        assert_eq!(env_config.custom_providers.len(), 1);

        let declaration = &env_config.custom_providers[0];
        let loaded_providers = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await
            .expect("custom providers should load successfully");

        assert_eq!(loaded_providers.len(), 2);

        let (provider1_source, provider1_def) = loaded_providers
            .get("provider1")
            .expect("provider1 should exist");
        assert_eq!(
            provider1_source,
            &SourceDir::local(providers_dir.canonicalize().unwrap())
        );
        assert_eq!(provider1_def.name, "simple provider");
        assert_eq!(
            provider1_def.description,
            "A simple custom provider for integration testing"
        );
        assert_eq!(provider1_def.variable_definitions.len(), 1);

        let (provider2_source, provider2_def) = loaded_providers
            .get("provider2")
            .expect("provider2 should exist");
        assert_eq!(
            provider2_source,
            &SourceDir::local(providers_dir.canonicalize().unwrap())
        );
        assert_eq!(provider2_def.name, "simple provider");
        assert_eq!(
            provider2_def.description,
            "A simple custom provider for integration testing"
        );
        assert_eq!(provider2_def.variable_definitions.len(), 1);
    }
}
