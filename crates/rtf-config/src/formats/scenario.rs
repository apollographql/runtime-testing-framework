use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    formats::{CustomProviderDeclaration, Result},
    providers::{command::CommandSection, file::Source},
    templating::{self, FileType, Template, TemplateContext},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path};

/// # Scenario Config
///
/// Configuration for a single test scenario to be executed as part of a test plan.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ScenarioConfig {
    /// The name of this scenario
    pub name: String,
    /// A brief description of the purpose / behaviour of this scenario
    pub description: String,
    /// Definitions for the required variables for templating this scenario
    #[serde(default, alias = "values")]
    // This alias is for backwards compatibility with the original name
    pub variable_definitions: Vec<VariableDefinition>,
    /// Custom provider declarations to load for this scenario
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderDeclaration>,
    /// The command to execute as this scenario
    #[serde(flatten)]
    pub command: CommandSection,
}

impl ScenarioConfig {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    /// Create an empty [ScenarioConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> ScenarioConfig {
        ScenarioConfig {
            name: Default::default(),
            description: Default::default(),
            variable_definitions: Default::default(),
            custom_providers: Default::default(),
            command: CommandSection::empty(),
        }
    }
}

impl Template for ScenarioConfig {
    fn has_pending_fields(&self) -> bool {
        self.command.has_pending_fields()
    }

    fn required_variables(&self) -> Vec<String> {
        self.command.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &Source,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut allowed_variables = allowed_variables.clone();
        allowed_variables.extend(self.variable_definitions.iter().map(|vd| &vd.name));

        let ctx = ctx.for_config_file(
            file_source,
            Some(FileType::Scenario),
            self.variable_definitions.iter(),
        );

        self.command
            .validate_context(path, &allowed_variables, file_source, &ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let ctx = ctx.for_config_file(
            source,
            Some(FileType::Scenario),
            self.variable_definitions.iter(),
        );

        self.command
            .try_template_nested(path, "command_section", source, &ctx)
    }
}

impl Check for ScenarioConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        self.command.try_check_nested(path, "command", ctx)
    }
}

impl CheckArrayDuplicates for ScenarioConfig {
    const BASE_PATH: &str = "scenario";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![
            (
                "variables",
                DedupArray::VariableDef(&mut self.variable_definitions),
            ),
            (
                "file_providers",
                DedupArray::Nfp(&mut self.command.file_providers),
            ),
        ]
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use crate::{
        formats::tests::{
            named_file_providers_with_fields, templatable_file_providers, variable_definitions,
        },
        templating::Field,
    };

    /// Create a ScenarioConfig for testing Template trait methods (has_pending_fields, required_variables)
    pub(crate) fn scenario_with_fields(
        fields: &[Field<String>],
        custom_providers: &[CustomProviderDeclaration],
    ) -> ScenarioConfig {
        ScenarioConfig {
            custom_providers: custom_providers.to_vec(),
            command: CommandSection {
                file_providers: named_file_providers_with_fields(fields),
                ..CommandSection::empty()
            },
            ..ScenarioConfig::empty()
        }
    }

    /// Create a ScenarioConfig for template testing
    pub(crate) fn templatable_scenario(
        variable_names: &[&str],
        scenario_fields: &[&str],
        custom_providers: &[CustomProviderDeclaration],
    ) -> ScenarioConfig {
        ScenarioConfig {
            custom_providers: custom_providers.to_vec(),
            variable_definitions: variable_definitions(variable_names),
            command: CommandSection {
                file_providers: templatable_file_providers(scenario_fields),
                ..CommandSection::empty()
            },
            ..ScenarioConfig::empty()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        formats::{
            scenario::test_helpers::{scenario_with_fields, templatable_scenario},
            tests::{
                assert_check_errors, assert_template_errors, expected_error_details, p, r,
                template_context,
            },
        },
        providers::{
            self,
            command::test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            file::{RawSource, Source},
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
    use std::{collections::HashMap, path::PathBuf};

    // An example scenario config to check parsing and templating
    const TEMPLATED_SCENARIO: &str = indoc!(
        r#"
        name: scenario
        description: a templated scenario
        variable_definitions:
          - name: foo
            description: a value foo
          - name: bar
            description: a value bar
            default: "bar"
        custom_providers:
          - kind: local
            relative_path: ../providers
            using:
              my_custom_provider: my_custom_provider.yaml
          - kind: github
            org: apollographql
            repo: test-providers
            path: /providers
            git_ref: main
            using:
              another_provider: another_provider.yaml
        command:
          name: scenario.sh
          kind: relative_path
          path: "{{ foo }}"
        file_providers:
          - name: file.txt
            env_var: FILE
            kind: relative_path
            path: "{{ bar }}"
    "#
    );

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

    #[test]
    fn parse_and_template() {
        let config: ScenarioConfig =
            serde_yaml::from_str(TEMPLATED_SCENARIO).expect("scenario config to parse");

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a determistic order for the assert_eq

        assert_eq!(res, &["bar", "foo"], "expected variables to match");
        assert_eq!(config.custom_providers.len(), 2);

        let cp = &config.custom_providers[0];
        assert_eq!(
            cp.source,
            RawSource::Local {
                relative_path: PathBuf::from("../providers")
            }
        );
        assert_eq!(cp.using.len(), 1);
        assert_eq!(
            cp.using.get("my_custom_provider").unwrap(),
            "my_custom_provider.yaml"
        );

        let cp = &config.custom_providers[1];
        assert_eq!(
            cp.source,
            RawSource::Github {
                org: "apollographql".to_string(),
                repo: "test-providers".to_string(),
                path: PathBuf::from("/providers"),
                git_ref: Some("main".to_string())
            }
        );
        assert_eq!(cp.using.len(), 1);
        assert_eq!(
            cp.using.get("another_provider").unwrap(),
            "another_provider.yaml"
        );
    }

    #[test_case(&[p("foo")], true; "single field is pending")]
    #[test_case(&[r("foo")], false; "single field is resolved")]
    #[test_case(&[p("field1"), p("field2")], true; "multiple fields pending is pending")]
    #[test_case(&[p("field1"), r("field2")], true; "multiple fields with single field pending is pending")]
    #[test_case(&[r("field1"), r("field2")], false; "multiple fields none pending is resolved")]
    #[test]
    fn has_pending_fields(fields: &[Field<String>], expected: bool) {
        let scenario = scenario_with_fields(fields, &[]);

        let res = scenario.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(&[p("foo")], &["foo"]; "single field is required")]
    #[test_case(&[r("foo")], &[]; "single field resolved requires no variables")]
    #[test_case(&[p("field1"), p("field2")], &["field1", "field2"]; "multiple fields pending requires variables")]
    #[test_case(&[p("field1"), r("field2")], &["field1"]; "multiple fields with single pending requires variables")]
    #[test_case(&[r("field1"), r("field2")], &[]; "multiple fields none pending requires no variables")]
    #[test]
    fn required_variables(fields: &[Field<String>], expected: &[&str]) {
        let scenario = scenario_with_fields(fields, &[]);

        let res = scenario.required_variables();
        assert_eq!(
            res, expected,
            "tests that required_variables has expected value"
        )
    }

    #[test_case(&["foo"]; "single variable")]
    #[test_case(&["foo", "bar"]; "multiple variables")]
    #[test_case(&["foo", "bar", "baz"]; "three variables")]
    #[test_case(&[]; "no variables")]
    #[test]
    fn try_template_succeeds(field_names: &[&str]) {
        let ctx = template_context(field_names);
        let mut scenario = templatable_scenario(field_names, field_names, &[]);

        let res = scenario.try_template(&mut Vec::new(), &Source::local("/"), &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test_case(&["foo"], &[], &["foo"], &["foo"]; "single variable provided and not defined")]
    #[test_case(&[], &["foo", "bar", "baz"], &["foo", "bar", "baz"], &["bar", "baz", "foo"]; "no variables provided and multiple defined")]
    #[test_case(&["foo", "bar", "baz"], &["foo", "bar"], &["foo", "bar", "baz"], &["baz"]; "multiple variables provided and one not defined")]
    #[test_case(&[], &["foo"], &["foo"], &["foo"]; "no variables provided but single variable defined")]
    #[test_case(&[], &["foo", "bar", "baz"], &["foo", "bar", "baz"], &["bar", "baz", "foo"]; "no variables provided but multiple variables defined")]
    #[test_case(&["foo", "bar"], &["foo", "bar", "baz"], &["foo", "bar", "baz"], &["baz"]; "one provided variable missing when multiple variables defined")]
    #[test]
    fn try_template_missing_variable_definitions(
        variables: &[&str],
        variable_defs: &[&str],
        scenario_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(variables);
        let mut scenario = templatable_scenario(variable_defs, scenario_fields, &[]);

        let (expected_err_messages, expected_err_paths) =
            expected_error_details(expected_err_fields, "command_section");

        assert_template_errors(
            &mut scenario,
            ctx,
            expected_err_messages,
            expected_err_paths,
        );
    }

    #[test]
    fn check_success() {
        let scenario = ScenarioConfig {
            command: cmd_with_inline_file(),
            ..ScenarioConfig::empty()
        };

        let ctx = Context::new();

        let res = scenario.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn check_command_errors() {
        let scenario = ScenarioConfig {
            command: cmd_with_required_file(),
            ..ScenarioConfig::empty()
        };

        let ctx = Context::new();

        assert_check_errors(scenario, &ctx, &[checks::ErrorKind::RequiredFileMissing]);
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

        let scenario_config_yaml = indoc!(
            r#"
            name: test scenario
            description: Scenario with custom providers
            variable_definitions: []
            custom_providers:
              - kind: local
                relative_path: providers
                using:
                  provider1: provider1.yaml
                  provider2: provider2.yaml
            command:
              name: scenario.sh
              kind: relative_path
              path: ./scenario.sh
            "#
        );

        let scenario_config: ScenarioConfig =
            serde_yaml::from_str(scenario_config_yaml).expect("scenario config to parse");

        assert_eq!(scenario_config.custom_providers.len(), 1);

        let declaration = &scenario_config.custom_providers[0];
        let loaded_providers = declaration
            .try_load_all(&Source::local(temp.path()), &Context::new())
            .await
            .expect("custom providers should load successfully");

        assert_eq!(loaded_providers.len(), 2);

        let (provider1_source, provider1_def) = loaded_providers
            .get("provider1")
            .expect("provider1 should exist");
        assert_eq!(
            provider1_source,
            &Source::local(providers_dir.canonicalize().unwrap())
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
            &Source::local(providers_dir.canonicalize().unwrap())
        );
        assert_eq!(provider2_def.name, "simple provider");
        assert_eq!(
            provider2_def.description,
            "A simple custom provider for integration testing"
        );
        assert_eq!(provider2_def.variable_definitions.len(), 1);
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
            .try_load_all(&Source::local(temp.path()), &Context::new())
            .await;

        assert!(res.is_err(), "expected error for missing file");
        let errors = res.unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "missing_provider");
        assert!(matches!(errors[0].1, providers::Error::Io(_)));
    }

    #[tokio::test]
    async fn custom_provider_try_load_all_invalid_yaml_errors() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        providers_dir
            .child("invalid.yaml")
            .write_str("not valid yaml: {{{]}")
            .unwrap();

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: HashMap::from([("invalid_provider".into(), "invalid.yaml".into())]),
        };

        let res = declaration
            .try_load_all(&Source::local(temp.path()), &Context::new())
            .await;

        assert!(res.is_err(), "expected error for invalid YAML");
        let errors = res.unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "invalid_provider");
        assert!(matches!(errors[0].1, providers::Error::Yaml(_)));
    }

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
            .try_load_all(&Source::local(temp.path()), &Context::new())
            .await;

        assert!(result.is_err(), "expected error for nested custom provider");
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
            .try_load_all(&Source::local(temp.path()), &Context::new())
            .await;

        assert!(
            result.is_err(),
            "expected errors for invalid and missing providers"
        );
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
}
