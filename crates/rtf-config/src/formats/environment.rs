//! Parsing of the environment provisioner config file format
use crate::{
    ValueDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray, duplicate_keys},
    context::ResolutionContext,
    formats::Result,
    providers::{command::CommandSection, file::Source},
    templating::{self, Template, TemplateValues},
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
    /// Definitions for the required values for templating this environment
    #[serde(default)]
    pub values: Vec<ValueDefinition>,
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
    /// Setup is only allowed to reference values that are declared in the values section of this
    /// config file.
    pub fn try_template_setup(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        values: &TemplateValues,
    ) -> templating::Result<()> {
        let allowed_values = values.for_config_file(source, self.values.iter());

        self.setup
            .command
            .try_template_nested(path, "setup", source, &allowed_values)
    }

    /// Try to template the teardown [CommandSection].
    ///
    /// Teardown is allowed to reference values that come from the output of setup in addition to
    /// the values decalered in the values section of this config file.
    pub fn try_template_teardown(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        values: &TemplateValues,
    ) -> templating::Result<()> {
        let allowed_values =
            values.for_config_file(source, self.values.iter().chain(self.setup.provides.iter()));

        self.teardown
            .try_template_nested(path, "teardown", source, &allowed_values)
    }

    /// Create an empty [EnvironmentConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> EnvironmentConfig {
        EnvironmentConfig {
            name: Default::default(),
            description: Default::default(),
            values: Vec::new(),
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

    fn required_values(&self) -> Vec<String> {
        let mut vals = self.setup.command.required_values();
        vals.extend(self.teardown.required_values());

        vals
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        values: &TemplateValues,
    ) -> templating::Result<()> {
        let mut errs =
            templating::ErrorBuilder::from(self.try_template_setup(path, source, values));
        errs.append(self.try_template_teardown(path, source, values));

        errs.into_result(())
    }
}

impl Check for EnvironmentConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        // Check that hard coded values and the ones coming from setup.provides are unique
        let all_values = self.values.iter().chain(self.setup.provides.iter());
        let duplicates = duplicate_keys(all_values, |v| &v.name);
        if !duplicates.is_empty() {
            errs.push(
                checks::ErrorKind::DuplicateValueNames,
                duplicates.join("\n"),
                path,
            );
        }

        // Check that each command is valid in isolation
        errs.append(self.setup.command.try_check_nested(path, "setup", src, ctx));
        errs.append(self.teardown.try_check_nested(path, "teardown", src, ctx));

        errs.into_result(())
    }
}

impl CheckArrayDuplicates for EnvironmentConfig {
    const BASE_PATH: &str = "environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![
            ("values", DedupArray::ValueDef(&mut self.values)),
            (
                "setup.provides",
                DedupArray::ValueDef(&mut self.setup.provides),
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
    /// Additional templating values that will be provided through the output of this command
    #[serde(default)]
    pub provides: Vec<ValueDefinition>,
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use crate::{
        formats::tests::{
            named_file_providers_with_fields, templatable_file_providers, value_definitions,
        },
        templating::Field,
    };

    /// Create an EnvironmentConfig for testing Template trait methods (has_pending_fields, required_values)
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
        value_names: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
    ) -> EnvironmentConfig {
        EnvironmentConfig {
            values: value_definitions(value_names),
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
                templatable_file_providers, template_values, value_definitions,
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

    /// Create an EnvironmentConfig for testing value scoping behavior
    /// Sets up predefined template fields that reference specific variable names
    fn environment_with_provides(available_vals: &[&str], provides: &[&str]) -> EnvironmentConfig {
        EnvironmentConfig {
            values: value_definitions(available_vals),
            setup: SetupSection {
                command: CommandSection {
                    env_vars: [("foo".to_uppercase(), Field::Pending("foo".to_string()))]
                        .into_iter()
                        .collect(),
                    file_providers: templatable_file_providers(&["setup-path"]),
                    ..CommandSection::empty()
                },
                provides: value_definitions(provides),
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
        values:
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

        let mut res = config.required_values();
        res.sort(); // Sorting so values are in a determistic order for the assert_eq
        assert_eq!(res, &["bar", "foo"], "expected values to match")
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

    #[test_case(&[p("setup1"), p("setup2")], &[p("teardown1"), p("teardown2")], &["setup1", "setup2", "teardown1", "teardown2"]; "both setup and both teardown pending requires values")]
    #[test_case(&[p("setup1"), p("setup2")], &[p("teardown1"), r("teardown2")], &["setup1", "setup2", "teardown1"]; "both setup and single teardown pending requires values")]
    #[test_case(&[p("setup1"), p("setup2")], &[r("teardown1"), r("teardown2")], &["setup1", "setup2"]; "both setup and no teardown pending requires values")]
    #[test_case(&[p("setup1"), r("setup2")], &[p("teardown1"), p("teardown2")], &["setup1", "teardown1", "teardown2"]; "single setup and both teardown pending requires values")]
    #[test_case(&[p("setup1"), r("setup2")], &[p("teardown1"), r("teardown2")], &["setup1", "teardown1"]; "single setup and single teardown pending requires values")]
    #[test_case(&[p("setup1"), r("setup2")], &[r("teardown1"), r("teardown2")], &["setup1"]; "single setup and no teardown pending requires values")]
    #[test_case(&[r("setup1"), r("setup2")], &[p("teardown1"), p("teardown2")], &["teardown1", "teardown2"]; "no setup and both teardown pending requires values")]
    #[test_case(&[r("setup1"), r("setup2")], &[p("teardown1"), r("teardown2")], &["teardown1"]; "no setup and single teardown pending requires values")]
    #[test_case(&[r("setup1"), r("setup2")], &[r("teardown1"), r("teardown2")], &[]; "no setup and no teardown pending requires no values")]
    #[test]
    fn required_values(
        setup_fields: &[Field<String>],
        teardown_fields: &[Field<String>],
        expected: &[&str],
    ) {
        let environment = environment_with_fields(setup_fields, teardown_fields);

        let res = environment.required_values();
        assert_eq!(
            res, expected,
            "tests that required_values has expected value"
        )
    }

    #[test_case(&["setup1", "setup2"], &["teardown1", "teardown2"]; "setup multi value and teardown multi value")]
    #[test_case(&["setup1", "setup2"], &["teardown1"]; "setup multi value and teardown single value")]
    #[test_case(&["setup1", "setup2"], &[]; "setup multi value and teardown no value")]
    #[test_case(&["setup1"], &["teardown1", "teardown2"]; "setup single value and teardown multi value")]
    #[test_case(&["setup1"], &["teardown1"]; "setup single value and teardown single value")]
    #[test_case(&["setup1"], &[]; "setup single value and teardown no value")]
    #[test_case(&[], &["teardown1", "teardown2"]; "setup no value and teardown multi value")]
    #[test_case(&[], &["teardown1"]; "setup no value and teardown single value")]
    #[test_case(&[], &[]; "setup no value and teardown no value")]
    #[test]
    fn try_template_succeeds(setup_fields: &[&str], teardown_fields: &[&str]) {
        let mut field_names: Vec<&str> = setup_fields.to_vec();
        field_names.extend_from_slice(teardown_fields);

        let values = template_values(field_names.as_slice());
        let mut environment =
            templatable_environment(field_names.as_slice(), setup_fields, teardown_fields);

        let res = environment.try_template(&mut Vec::new(), &Source::local("/"), &values);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    /// Helper function for asserting template errors are as expected
    fn assert_env_template_errors(
        environment: &mut EnvironmentConfig,
        values: TemplateValues,
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
            values,
            expected_err_messages,
            expected_err_paths,
        );
    }

    #[test_case(&["missing"], &["setup"], &["setup"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["setup1", "setup2"], &["setup1", "setup2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["setup1", "missing2"], &["setup1", "setup2"], &["setup2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but value not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but values not provided")]
    #[test]
    fn try_template_setup_missing_value_definitions(
        value_defs: &[&str],
        setup_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let values = template_values(&["setup", "setup1", "setup2"]);
        let mut environment = templatable_environment(value_defs, setup_fields, &[]);

        assert_env_template_errors(&mut environment, values, expected_err_fields, &[]);
    }

    #[test_case(&["missing"], &["teardown"], &["teardown"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["teardown1", "teardown2"], &["teardown1", "teardown2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["teardown1", "missing2"], &["teardown1", "teardown2"], &["teardown2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but value not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but values not provided")]
    #[test]
    fn try_template_teardown_missing_value_definitions(
        value_defs: &[&str],
        teardown_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let values = template_values(&["teardown", "teardown1", "teardown2"]);
        let mut environment = templatable_environment(value_defs, &[], teardown_fields);

        assert_env_template_errors(&mut environment, values, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_value_definitions() {
        let values = template_values(&["setup", "teardown"]);
        let mut environment = templatable_environment(&[], &["setup"], &["teardown"]);

        assert_env_template_errors(&mut environment, values, &["setup"], &["teardown"]);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_values_not_provided() {
        let values = template_values(&[]);
        let mut environment =
            templatable_environment(&["setup", "teardown"], &["setup"], &["teardown"]);

        assert_env_template_errors(&mut environment, values, &["setup"], &["teardown"]);
    }

    /// Tests that setup cannot access values from setup.provides.
    /// Setup can only access values declared in the top-level `values` section.
    #[test]
    fn try_template_setup_cannot_access_provides_values() {
        // Setup a config where values are only defined in setup.provides, not in top-level values
        let mut config = environment_with_provides(&[], &["foo", "setup-path"]);

        // Values exist in the map but are only defined in provides
        let values = template_values(&["foo", "setup-path"]);

        let res = config.try_template_setup(&mut Vec::new(), &Source::local("/"), &values);

        // Setup should fail because it cannot access provides values
        assert!(
            res.is_err(),
            "expected setup to fail when accessing provides values"
        );

        let errors = res.unwrap_err();
        let error_messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();

        // Should have errors for both values that setup tried to access from provides
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

    /// Tests that teardown can access values from setup.provides.
    /// Teardown can access values from both top-level `values` and `setup.provides`.
    #[test]
    fn try_template_teardown_can_access_provides_values() {
        // Setup a config where values are only defined in setup.provides, not in top-level values
        let mut config = environment_with_provides(&[], &["bar", "teardown-path"]);

        // Values exist in the map and are defined in provides
        let values = template_values(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &Source::local("/"), &values);

        // Teardown should succeed because it can access provides values
        assert!(
            res.is_ok(),
            "expected teardown to succeed when accessing provides values, got {res:?}"
        );
    }

    /// Tests that setup can access values from top-level values.
    /// Setup can access values declared in the top-level `values` section.
    #[test]
    fn try_template_setup_can_access_top_level_values() {
        // Setup a config where values are defined in top-level values
        let mut config = environment_with_provides(&["foo", "setup-path"], &[]);

        // Values exist in the map and are defined in top-level values
        let values: HashMap<String, Scalar> = [("foo", "a"), ("setup-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let res = config.try_template_setup(
            &mut Vec::new(),
            &Source::local("/"),
            &TemplateValues::new_stubbed(values),
        );

        // Setup should succeed because it can access top-level values
        assert!(
            res.is_ok(),
            "expected setup to succeed when accessing top-level values, got {res:?}"
        );
    }

    /// Tests that teardown can access values from top-level values.
    /// Teardown can access values from both top-level `values` and `setup.provides`.
    #[test]
    fn try_template_teardown_can_access_top_level_values() {
        // Setup a config where values are defined in top-level values
        let mut config = environment_with_provides(&["bar", "teardown-path"], &[]);

        // Values exist in the map and are defined in top-level values
        let values = template_values(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &Source::local("/"), &values);

        // Teardown should succeed because it can access top-level values
        assert!(
            res.is_ok(),
            "expected teardown to succeed when accessing top-level values, got {res:?}"
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
        let src = Source::Local {
            abs_path: "/".into(),
        };

        let res = environment.try_check(&mut Vec::new(), &src, &ctx);
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
        &[ErrorKind::DuplicateValueNames];
        "duplicate provides value"
    )]
    #[test_case(
        cmd_with_required_file(),
        cmd_with_required_file(),
        &["foo", "bar"],
        &[ErrorKind::DuplicateValueNames, ErrorKind::RequiredFileMissing, ErrorKind::RequiredFileMissing];
        "setup and teardown and duplicate values"
    )]
    #[test]
    fn try_check_errors(
        setup_command: CommandSection,
        teardown_command: CommandSection,
        provides: &[&str],
        expected_err_kinds: &[ErrorKind],
    ) {
        let environment = EnvironmentConfig {
            values: value_definitions(provides),
            setup: SetupSection {
                command: setup_command,
                provides: value_definitions(provides),
            },
            teardown: teardown_command,
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();
        let src = Source::Local {
            abs_path: "/".into(),
        };

        assert_check_errors(environment, &src, &ctx, expected_err_kinds);
    }
}
