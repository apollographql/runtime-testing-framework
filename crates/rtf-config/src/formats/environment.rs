//! Parsing of the environment provisioner config file format
use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray, duplicate_keys},
    context::ResolutionContext,
    formats::Result,
    providers::{command::CommandSection, file::Source},
    templating::{self, Template, TemplateVariables},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

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

    /// Try to template the setup [CommandSection].
    ///
    /// Setup is only allowed to reference variables that are declared in the variables section of this
    /// config file.
    pub fn try_template_setup(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        variables: &TemplateVariables,
    ) -> templating::Result<()> {
        let allowed_variables = variables.for_config_file(source, self.variable_definitions.iter());

        self.setup
            .command
            .try_template_nested(path, "setup", source, &allowed_variables)
    }

    /// Try to template the teardown [CommandSection].
    ///
    /// Teardown is allowed to reference variables that come from the output of setup in addition to
    /// the variables decalered in the variables section of this config file.
    pub fn try_template_teardown(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        variables: &TemplateVariables,
    ) -> templating::Result<()> {
        let allowed_variables = variables.for_config_file(
            source,
            self.variable_definitions
                .iter()
                .chain(self.setup.provides.iter()),
        );

        self.teardown
            .try_template_nested(path, "teardown", source, &allowed_variables)
    }

    /// Create an empty [EnvironmentConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> EnvironmentConfig {
        EnvironmentConfig {
            name: Default::default(),
            description: Default::default(),
            variable_definitions: Vec::new(),
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

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        variables: &TemplateVariables,
    ) -> templating::Result<()> {
        let mut errs =
            templating::ErrorBuilder::from(self.try_template_setup(path, source, variables));
        errs.append(self.try_template_teardown(path, source, variables));

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
    ) -> EnvironmentConfig {
        EnvironmentConfig {
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
    ) -> EnvironmentConfig {
        EnvironmentConfig {
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
                templatable_file_providers, template_variables, variable_definitions,
            },
        },
        providers::command::{
            CommandSection,
            test_helpers::{cmd_with_inline_file, cmd_with_required_file},
        },
        templating::{Field, Scalar},
    };
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::collections::HashMap;

    /// Create an EnvironmentConfig for testing variable scoping behavior
    /// Sets up predefined template fields that reference specific variable names
    fn environment_with_provides(available_vals: &[&str], provides: &[&str]) -> EnvironmentConfig {
        EnvironmentConfig {
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
        assert_eq!(res, &["bar", "foo"], "expected variables to match")
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
        let environment = environment_with_fields(&[setup_field], &[teardown_field]);

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
        let environment = environment_with_fields(setup_fields, teardown_fields);

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

        let variables = template_variables(field_names.as_slice());
        let mut environment =
            templatable_environment(field_names.as_slice(), setup_fields, teardown_fields);

        let res = environment.try_template(&mut Vec::new(), &Source::local("/"), &variables);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    /// Helper function for asserting template errors are as expected
    fn assert_env_template_errors(
        environment: &mut EnvironmentConfig,
        variables: TemplateVariables,
        expected_setup_err_fields: &[&str],
        expected_teardown_err_fields: &[&str],
    ) {
        let (mut expected_err_messages, mut expected_err_paths) =
            expected_error_details(expected_setup_err_fields, "setup");
        let (expected_messages, expected_paths) =
            expected_error_details(expected_teardown_err_fields, "teardown");
        expected_err_messages.extend(expected_messages);
        expected_err_paths.extend(expected_paths);

        assert_template_errors(
            environment,
            variables,
            expected_err_messages,
            expected_err_paths,
        );
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
        let variables = template_variables(&["setup", "setup1", "setup2"]);
        let mut environment = templatable_environment(variable_defs, setup_fields, &[]);

        assert_env_template_errors(&mut environment, variables, expected_err_fields, &[]);
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
        let variables = template_variables(&["teardown", "teardown1", "teardown2"]);
        let mut environment = templatable_environment(variable_defs, &[], teardown_fields);

        assert_env_template_errors(&mut environment, variables, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variable_definitions() {
        let variables = template_variables(&["setup", "teardown"]);
        let mut environment = templatable_environment(&[], &["setup"], &["teardown"]);

        assert_env_template_errors(&mut environment, variables, &["setup"], &["teardown"]);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variables_not_provided() {
        let variables = template_variables(&[]);
        let mut environment =
            templatable_environment(&["setup", "teardown"], &["setup"], &["teardown"]);

        assert_env_template_errors(&mut environment, variables, &["setup"], &["teardown"]);
    }

    /// Tests that setup cannot access variables from setup.provides.
    /// Setup can only access variables declared in the top-level `variables` section.
    #[test]
    fn try_template_setup_cannot_access_provides_variables() {
        // Setup a config where variables are only defined in setup.provides, not in top-level variables
        let mut config = environment_with_provides(&[], &["foo", "setup-path"]);

        // Variables exist in the map but are only defined in provides
        let variables = template_variables(&["foo", "setup-path"]);

        let res = config.try_template_setup(&mut Vec::new(), &Source::local("/"), &variables);

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
        let mut config = environment_with_provides(&[], &["bar", "teardown-path"]);

        // Variables exist in the map and are defined in provides
        let variables = template_variables(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &Source::local("/"), &variables);

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
        let mut config = environment_with_provides(&["foo", "setup-path"], &[]);

        // Variables exist in the map and are defined in top-level variables
        let variables: HashMap<String, Scalar> = [("foo", "a"), ("setup-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let res = config.try_template_setup(
            &mut Vec::new(),
            &Source::local("/"),
            &TemplateVariables::new_stubbed(variables),
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
        let mut config = environment_with_provides(&["bar", "teardown-path"], &[]);

        // Variables exist in the map and are defined in top-level variables
        let variables = template_variables(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &Source::local("/"), &variables);

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
}
