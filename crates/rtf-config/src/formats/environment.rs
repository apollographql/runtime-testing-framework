//! Parsing of the environment provisioner config file format
use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray, duplicate_keys},
    context::ResolutionContext,
    formats::{CustomProviderDeclaration, Result},
    providers::{command::CommandSection, file::Source},
    templating::{self, FileType, Template, TemplateContext},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path};

/// # Environment Config
///
/// Configuration for preparing and cleaning up the test environment as part of a test plan.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct EnvironmentConfig {
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
    /// The command to execute to prepare the environment for executing the test scenario
    pub setup: SetupSection,
    /// The command to execute to clean up the environment after executing the test scenario
    pub teardown: CommandSection,
}

impl EnvironmentConfig {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    fn ctx_for_setup(&self, source: &Source, ctx: &TemplateContext) -> TemplateContext {
        ctx.for_config_file(
            source,
            Some(FileType::Environment),
            self.variable_definitions.iter(),
        )
    }

    fn ctx_for_teardown(&self, source: &Source, ctx: &TemplateContext) -> TemplateContext {
        ctx.for_config_file(
            source,
            Some(FileType::Environment),
            self.variable_definitions
                .iter()
                .chain(self.setup.provides.iter()),
        )
    }

    /// Try to template the setup [CommandSection].
    ///
    /// Setup is only allowed to reference variables that are declared in the variables section of this
    /// config file.
    pub fn try_template_setup(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        self.setup.command.try_template_nested(
            path,
            "setup",
            source,
            &self.ctx_for_setup(source, ctx),
        )
    }

    /// Try to template the teardown [CommandSection].
    ///
    /// Teardown is allowed to reference variables that come from the output of setup in addition to
    /// the variables decalered in the variables section of this config file.
    pub fn try_template_teardown(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        self.teardown.try_template_nested(
            path,
            "teardown",
            source,
            &self.ctx_for_teardown(source, ctx),
        )
    }

    /// Create an empty [EnvironmentConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> EnvironmentConfig {
        EnvironmentConfig {
            name: Default::default(),
            description: Default::default(),
            variable_definitions: Vec::new(),
            custom_providers: Default::default(),
            setup: SetupSection {
                command: CommandSection::empty(),
                provides: Vec::new(),
            },
            teardown: CommandSection::empty(),
        }
    }
}

impl Template for EnvironmentConfig {
    fn has_pending_fields(&self) -> bool {
        self.setup.command.has_pending_fields() || self.teardown.has_pending_fields()
    }

    fn required_variables(&self) -> Vec<String> {
        let mut vals = self.setup.command.required_variables();
        vals.extend(self.teardown.required_variables());

        vals
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        source: &Source,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut allowed_variables = allowed_variables.clone();
        allowed_variables.extend(self.variable_definitions.iter().map(|vd| &vd.name));

        let mut errs = templating::ErrorBuilder::from(self.setup.command.validate_context_nested(
            path,
            "setup",
            &allowed_variables,
            source,
            &self.ctx_for_setup(source, ctx),
        ));

        errs.append(self.teardown.validate_context_nested(
            path,
            "teardown",
            &allowed_variables,
            source,
            &self.ctx_for_teardown(source, ctx),
        ));

        errs.into_result(())
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.try_template_setup(path, source, ctx));
        errs.append(self.try_template_teardown(path, source, ctx));

        errs.into_result(())
    }
}

impl Check for EnvironmentConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        // Check that hard coded variables and the ones coming from setup.provides are unique
        let all_variables = self
            .variable_definitions
            .iter()
            .chain(self.setup.provides.iter());
        let duplicates = duplicate_keys(all_variables, |v| &v.name);
        if !duplicates.is_empty() {
            errs.push(
                checks::ErrorKind::DuplicateVariableNames,
                duplicates.join("\n"),
                path,
            );
        }

        // Check that each command is valid in isolation
        errs.append(self.setup.command.try_check_nested(path, "setup", ctx));
        errs.append(self.teardown.try_check_nested(path, "teardown", ctx));

        errs.into_result(())
    }
}

impl CheckArrayDuplicates for EnvironmentConfig {
    const BASE_PATH: &str = "environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![
            (
                "variables",
                DedupArray::VariableDef(&mut self.variable_definitions),
            ),
            (
                "setup.provides",
                DedupArray::VariableDef(&mut self.setup.provides),
            ),
            (
                "setup.file_providers",
                DedupArray::Nfp(&mut self.setup.command.file_providers),
            ),
            (
                "teardown.file_providers",
                DedupArray::Nfp(&mut self.teardown.file_providers),
            ),
        ]
    }
}

/// # Setup Command Section
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct SetupSection {
    #[serde(flatten)]
    pub command: CommandSection,
    /// Additional templating variables that will be provided through the output of this command
    #[serde(default)]
    pub provides: Vec<VariableDefinition>,
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

    /// Create an EnvironmentConfig for testing Template trait methods (has_pending_fields, required_variables)
    pub(crate) fn environment_with_fields(
        setup_fields: &[Field<String>],
        teardown_fields: &[Field<String>],
        custom_providers: &[CustomProviderDeclaration],
    ) -> EnvironmentConfig {
        EnvironmentConfig {
            custom_providers: custom_providers.to_vec(),
            setup: SetupSection {
                command: CommandSection {
                    file_providers: named_file_providers_with_fields(setup_fields),
                    ..CommandSection::empty()
                },
                provides: Vec::new(),
            },
            teardown: CommandSection {
                file_providers: named_file_providers_with_fields(teardown_fields),
                ..CommandSection::empty()
            },
            ..EnvironmentConfig::empty()
        }
    }

    /// Create a test EnvironmentConfig for template tests
    pub(crate) fn templatable_environment(
        variable_names: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        custom_providers: &[CustomProviderDeclaration],
    ) -> EnvironmentConfig {
        EnvironmentConfig {
            custom_providers: custom_providers.to_vec(),
            variable_definitions: variable_definitions(variable_names),
            setup: SetupSection {
                command: CommandSection {
                    file_providers: templatable_file_providers(setup_fields),
                    ..CommandSection::empty()
                },
                provides: Vec::new(),
            },
            teardown: CommandSection {
                file_providers: templatable_file_providers(teardown_fields),
                ..CommandSection::empty()
            },
            ..EnvironmentConfig::empty()
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        checks::ErrorKind,
        context::Context,
        formats::{
            environment::test_helpers::{environment_with_fields, templatable_environment},
            tests::{
                assert_check_errors, assert_template_errors, expected_error_details, p, r,
                templatable_file_providers, template_context, variable_definitions,
            },
        },
        providers::{
            self,
            command::{
                CommandSection,
                test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            },
            file::{RawSource, Source},
            test_helpers::create_temp_dir_with_file,
        },
        templating::{Field, Scalar},
    };
    use assert_fs::{
        fixture::PathChild,
        prelude::{FileWriteStr, PathCreateDir},
    };
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::{collections::HashMap, path::PathBuf};

    /// Create an EnvironmentConfig for testing variable scoping behavior
    /// Sets up predefined template fields that reference specific variable names
    fn environment_with_provides(
        available_vals: &[&str],
        provides: &[&str],
        custom_providers: &[CustomProviderDeclaration],
    ) -> EnvironmentConfig {
        EnvironmentConfig {
            custom_providers: custom_providers.to_vec(),
            variable_definitions: variable_definitions(available_vals),
            setup: SetupSection {
                command: CommandSection {
                    env_vars: [("foo".to_uppercase(), Field::Pending("foo".to_string()))]
                        .into_iter()
                        .collect(),
                    file_providers: templatable_file_providers(&["setup-path"]),
                    ..CommandSection::empty()
                },
                provides: variable_definitions(provides),
            },
            teardown: CommandSection {
                env_vars: [("bar".to_uppercase(), Field::Pending("bar".to_string()))]
                    .into_iter()
                    .collect(),
                file_providers: templatable_file_providers(&["teardown-path"]),
                ..CommandSection::empty()
            },
            ..EnvironmentConfig::empty()
        }
    }

    // An example environment config to check parsing and templating
    const TEMPLATED_ENVIRONMENT: &str = indoc!(
        r#"
        name: environment
        description: a templated environment
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
        setup:
          command:
            name: setup.sh
            kind: relative_path
            path: "/setup.sh"
          file_providers:
            - name: foo.txt
              env_var: FOO
              kind: relative_path
              path: "{{ foo }}"
          provides:
            - name: baz
              description: a value baz
        teardown:
          command:
            name: teardown.sh
            kind: relative_path
            path: "/teardown.sh"
          file_providers:
            - name: bar.txt
              env_var: BAR
              kind: relative_path
              path: "{{ bar }}"
    "#
    );

    #[test]
    fn parse_success() {
        let config: EnvironmentConfig =
            serde_yaml::from_str(TEMPLATED_ENVIRONMENT).expect("environment config to parse");

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
            "my_custom_provider.yaml",
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
            "another_provider.yaml",
        );
    }

    #[test_case(p("setup"), p("teardown"), true; "setup and teardown pending is pending")]
    #[test_case(p("setup"), r("teardown"), true; "setup pending and teardown resolved is pending")]
    #[test_case(r("setup"), p("teardown"), true; "setup resolved and teardown pending is pending")]
    #[test_case(r("setup"), r("teardown"), false; "setup resolved and teardown resolved is resolved")]
    #[test]
    fn has_pending_fields(
        setup_field: Field<String>,
        teardown_field: Field<String>,
        expected: bool,
    ) {
        let environment = environment_with_fields(&[setup_field], &[teardown_field], &[]);

        let res = environment.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

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

    #[test_case(&["setup1", "setup2"], &["teardown1", "teardown2"]; "setup multi variable and teardown multi variable")]
    #[test_case(&["setup1", "setup2"], &["teardown1"]; "setup multi variable and teardown single variable")]
    #[test_case(&["setup1", "setup2"], &[]; "setup multi variable and teardown no variable")]
    #[test_case(&["setup1"], &["teardown1", "teardown2"]; "setup single variable and teardown multi variable")]
    #[test_case(&["setup1"], &["teardown1"]; "setup single variable and teardown single variable")]
    #[test_case(&["setup1"], &[]; "setup single variable and teardown no variable")]
    #[test_case(&[], &["teardown1", "teardown2"]; "setup no variable and teardown multi variable")]
    #[test_case(&[], &["teardown1"]; "setup no variable and teardown single variable")]
    #[test_case(&[], &[]; "setup no variable and teardown no variable")]
    #[test]
    fn try_template_succeeds(setup_fields: &[&str], teardown_fields: &[&str]) {
        let mut field_names: Vec<&str> = setup_fields.to_vec();
        field_names.extend_from_slice(teardown_fields);

        let ctx = template_context(field_names.as_slice());
        let mut environment =
            templatable_environment(field_names.as_slice(), setup_fields, teardown_fields, &[]);

        let res = environment.try_template(&mut Vec::new(), &Source::local("/"), &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    /// Helper function for asserting template errors are as expected
    fn assert_env_template_errors(
        environment: &mut EnvironmentConfig,
        ctx: TemplateContext,
        expected_setup_err_fields: &[&str],
        expected_teardown_err_fields: &[&str],
    ) {
        let (mut expected_err_messages, mut expected_err_paths) =
            expected_error_details(expected_setup_err_fields, "setup");
        let (expected_messages, expected_paths) =
            expected_error_details(expected_teardown_err_fields, "teardown");
        expected_err_messages.extend(expected_messages);
        expected_err_paths.extend(expected_paths);

        assert_template_errors(environment, ctx, expected_err_messages, expected_err_paths);
    }

    #[test_case(&["missing"], &["setup"], &["setup"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["setup1", "setup2"], &["setup1", "setup2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["setup1", "missing2"], &["setup1", "setup2"], &["setup2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_setup_missing_variable_definitions(
        variable_defs: &[&str],
        setup_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["setup", "setup1", "setup2"]);
        let mut environment = templatable_environment(variable_defs, setup_fields, &[], &[]);

        assert_env_template_errors(&mut environment, ctx, expected_err_fields, &[]);
    }

    #[test_case(&["missing"], &["teardown"], &["teardown"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["teardown1", "teardown2"], &["teardown1", "teardown2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["teardown1", "missing2"], &["teardown1", "teardown2"], &["teardown2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_teardown_missing_variable_definitions(
        variable_defs: &[&str],
        teardown_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["teardown", "teardown1", "teardown2"]);
        let mut environment = templatable_environment(variable_defs, &[], teardown_fields, &[]);

        assert_env_template_errors(&mut environment, ctx, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variable_definitions() {
        let ctx = template_context(&["setup", "teardown"]);
        let mut environment = templatable_environment(&[], &["setup"], &["teardown"], &[]);

        assert_env_template_errors(&mut environment, ctx, &["setup"], &["teardown"]);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variables_not_provided() {
        let ctx = template_context(&[]);
        let mut environment =
            templatable_environment(&["setup", "teardown"], &["setup"], &["teardown"], &[]);

        assert_env_template_errors(&mut environment, ctx, &["setup"], &["teardown"]);
    }

    /// Tests that setup cannot access variables from setup.provides.
    /// Setup can only access variables declared in the top-level `variables` section.
    #[test]
    fn try_template_setup_cannot_access_provides_variables() {
        // Setup a config where variables are only defined in setup.provides, not in top-level variables
        let mut config = environment_with_provides(&[], &["foo", "setup-path"], &[]);

        // Variables exist in the map but are only defined in provides
        let ctx = template_context(&["foo", "setup-path"]);

        let res = config.try_template_setup(&mut Vec::new(), &Source::local("/"), &ctx);

        // Setup should fail because it cannot access provides variables
        assert!(
            res.is_err(),
            "expected setup to fail when accessing provides variables"
        );

        let errors = res.unwrap_err();
        let error_messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();

        // Should have errors for both variables that setup tried to access from provides
        assert!(
            error_messages.contains(&"foo".to_string()),
            "expected error messages to contain foo, got, {:?}",
            error_messages
        );
        assert!(
            error_messages.contains(&"setup-path".to_string()),
            "expected error messages to contain setup-path, got, {:?}",
            error_messages
        );
    }

    /// Tests that teardown can access variables from setup.provides.
    /// Teardown can access variables from both top-level `variables` and `setup.provides`.
    #[test]
    fn try_template_teardown_can_access_provides_variables() {
        // Setup a config where variables are only defined in setup.provides, not in top-level variables
        let mut config = environment_with_provides(&[], &["bar", "teardown-path"], &[]);

        // Variables exist in the map and are defined in provides
        let ctx = template_context(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &Source::local("/"), &ctx);

        // Teardown should succeed because it can access provides variables
        assert!(
            res.is_ok(),
            "expected teardown to succeed when accessing provides variables, got {res:?}"
        );
    }

    /// Tests that setup can access variables from top-level variables.
    /// Setup can access variables declared in the top-level `variables` section.
    #[test]
    fn try_template_setup_can_access_top_level_variables() {
        // Setup a config where variables are defined in top-level variables
        let mut config = environment_with_provides(&["foo", "setup-path"], &[], &[]);

        // Variables exist in the map and are defined in top-level variables
        let variables: HashMap<String, Scalar> = [("foo", "a"), ("setup-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let res = config.try_template_setup(
            &mut Vec::new(),
            &Source::local("/"),
            &TemplateContext::new_stubbed(variables),
        );

        // Setup should succeed because it can access top-level variables
        assert!(
            res.is_ok(),
            "expected setup to succeed when accessing top-level variables, got {res:?}"
        );
    }

    /// Tests that teardown can access variables from top-level variables.
    /// Teardown can access variables from both top-level `variables` and `setup.provides`.
    #[test]
    fn try_template_teardown_can_access_top_level_variables() {
        // Setup a config where variables are defined in top-level variables
        let mut config = environment_with_provides(&["bar", "teardown-path"], &[], &[]);

        // Variables exist in the map and are defined in top-level variables
        let ctx = template_context(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &Source::local("/"), &ctx);

        // Teardown should succeed because it can access top-level variables
        assert!(
            res.is_ok(),
            "expected teardown to succeed when accessing top-level variables, got {res:?}"
        );
    }

    // The success test is here to complete the matrix of failure tests below (i.e. no failure)
    // It shows all parts of the env config that *could* fail not failing. In reality we could
    // just use an "empty" config here and get the same result but this is a more illustrative
    // example
    #[test]
    fn check_success() {
        let environment = EnvironmentConfig {
            setup: SetupSection {
                command: cmd_with_inline_file(),
                provides: Vec::new(),
            },
            teardown: cmd_with_inline_file(),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let res = environment.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
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
        CommandSection::empty(),
        CommandSection::empty(),
        &["foo"],
        &[ErrorKind::DuplicateVariableNames];
        "duplicate provides variable"
    )]
    #[test_case(
        cmd_with_required_file(),
        cmd_with_required_file(),
        &["foo", "bar"],
        &[ErrorKind::DuplicateVariableNames, ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "setup and teardown and duplicate variables"
    )]
    #[test]
    fn try_check_errors(
        setup_command: CommandSection,
        teardown_command: CommandSection,
        provides: &[&str],
        expected_err_kinds: &[ErrorKind],
    ) {
        let environment = EnvironmentConfig {
            variable_definitions: variable_definitions(provides),
            setup: SetupSection {
                command: setup_command,
                provides: variable_definitions(provides),
            },
            teardown: teardown_command,
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        assert_check_errors(environment, &ctx, expected_err_kinds);
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
            .try_load_all(&Source::local(temp.path()), &Context::new())
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
            .try_load_all(&Source::local(temp.path()), &Context::new())
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

        let env_config: EnvironmentConfig =
            serde_yaml::from_str(env_config_yaml).expect("environment config to parse");

        assert_eq!(env_config.custom_providers.len(), 1);

        let declaration = &env_config.custom_providers[0];
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
}
