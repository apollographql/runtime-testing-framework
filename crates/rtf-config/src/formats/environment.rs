//! Parsing of the environment provisioner config file format
use crate::{
    ValueDefinition,
    checks::{self, Check, duplicate_keys},
    context::ResolutionContext,
    formats::{Result, values_for_config_file},
    providers::{command::CommandSection, file::Source},
    templating::{self, Scalar, Template},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

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
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let definitions = self.values.iter();
        let allowed_values = values_for_config_file(values, definitions);

        self.setup
            .command
            .try_template_nested(path, "setup", &allowed_values)
    }

    /// Try to template the teardown [CommandSection].
    ///
    /// Teardown is allowed to reference values that come from the output of setup in addition to
    /// the values decalered in the values section of this config file.
    pub fn try_template_teardown(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let definitions = self.values.iter().chain(self.setup.provides.iter());
        let allowed_values = values_for_config_file(values, definitions);

        self.teardown
            .try_template_nested(path, "teardown", &allowed_values)
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
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.try_template_setup(path, values));
        errs.append(self.try_template_teardown(path, values));

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
mod tests {
    use super::*;
    use crate::{
        context::Context,
        providers::file::{FileProvider, NamedFileProvider, RelativeFile},
        templating::{ErrorKind, Field},
    };
    use indoc::indoc;
    use simple_test_case::{dir_cases, test_case};
    use simple_txtar::Archive;
    use std::path::PathBuf;

    // Helper functions

    /// Load a txtar [Archive] from the given file content and print the top level comment if there
    /// is one before returning it.
    fn load_archive(content: &str) -> Archive {
        let arr = Archive::from(content);
        let comment = arr.comment();
        if !comment.is_empty() {
            println!("{}", comment.trim());
        }

        arr
    }

    /// Read the requested file from the archive, panicking if it is missing
    fn get_file<'a>(arr: &'a Archive, fname: &str) -> &'a str {
        match arr.get(fname) {
            Some(f) => f.content.trim(),
            None => {
                panic!("required txtar file section {fname:?} was missing");
            }
        }
    }

    /// Return a pending field
    fn p(name: &str) -> Field<String> {
        Field::Pending(name.to_string())
    }

    /// Return a resolved field
    fn r(name: &str) -> Field<String> {
        Field::Resolved(name.to_string())
    }

    /// Return a NamedFileProvider with a field
    fn named_file_provider_with_field(name: &str, f: Field<String>) -> NamedFileProvider {
        NamedFileProvider {
            name: name.to_string(),
            env_var: name.to_ascii_uppercase(),
            provider: FileProvider::RelativePath(RelativeFile { path: f, src: None }),
        }
    }

    /// Create an EnvironmentConfig for testing Template trait methods (has_pending_fields, required_values)
    fn template_trait_test_config(
        setup_field_1: Field<String>,
        setup_field_2: Field<String>,
        teardown_field_1: Field<String>,
        teardown_field_2: Field<String>,
    ) -> EnvironmentConfig {
        EnvironmentConfig {
            setup: SetupSection {
                command: CommandSection {
                    file_providers: vec![
                        named_file_provider_with_field("setup1", setup_field_1),
                        named_file_provider_with_field("setup2", setup_field_2),
                    ],
                    ..CommandSection::empty()
                },
                provides: Vec::new(),
            },
            teardown: CommandSection {
                file_providers: vec![
                    named_file_provider_with_field("teardown1", teardown_field_1),
                    named_file_provider_with_field("teardown2", teardown_field_2),
                ],
                ..CommandSection::empty()
            },
            ..EnvironmentConfig::empty()
        }
    }

    /// Create a HashMap of values from string names (each name maps to itself as a Scalar::String)
    fn value_map(value_names: &[&str]) -> HashMap<String, Scalar> {
        value_names
            .iter()
            .map(|&name| (name.to_string(), Scalar::String(name.to_string())))
            .collect()
    }

    /// Create ValueDefinitions from string names with default description
    fn value_definitions(value_names: &[&str]) -> Vec<ValueDefinition> {
        value_names
            .iter()
            .map(|&name| ValueDefinition {
                name: name.to_string(),
                description: "description".to_string(),
                default: None,
            })
            .collect()
    }

    /// Create NamedFileProviders with pending fields from string names
    fn file_providers_from_names(field_names: &[&str]) -> Vec<NamedFileProvider> {
        field_names
            .iter()
            .map(|name| named_file_provider_with_field(name, p(name)))
            .collect()
    }

    /// Create a test EnvironmentConfig with specified setup and teardown field names
    /// All field names are added as both value definitions and pending template fields
    fn test_environment_config(
        value_names: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
    ) -> EnvironmentConfig {
        EnvironmentConfig {
            values: value_definitions(value_names),
            setup: SetupSection {
                command: CommandSection {
                    file_providers: file_providers_from_names(setup_fields),
                    ..CommandSection::empty()
                },
                provides: Vec::new(),
            },
            teardown: CommandSection {
                file_providers: file_providers_from_names(teardown_fields),
                ..CommandSection::empty()
            },
            ..EnvironmentConfig::empty()
        }
    }

    /// Generate expected error details for fields
    fn expected_error_details(
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

    /// Create a CommandSection with both env vars and file providers for testing scoping
    fn cmd_section_with_scoping_fields(name: &str, env_var: &str) -> CommandSection {
        CommandSection {
            env_vars: [(env_var.to_uppercase(), Field::Pending(env_var.to_string()))]
                .into_iter()
                .collect(),
            file_providers: file_providers_from_names(&[name]),
            ..CommandSection::empty()
        }
    }

    /// Create an EnvironmentConfig for testing value scoping behavior
    /// Sets up predefined template fields that reference specific variable names
    fn scoping_test_config(available_vals: &[&str], provides: &[&str]) -> EnvironmentConfig {
        EnvironmentConfig {
            values: value_definitions(available_vals),
            setup: SetupSection {
                command: cmd_section_with_scoping_fields("setup-path", "foo"),
                provides: value_definitions(provides),
            },
            teardown: cmd_section_with_scoping_fields("teardown-path", "bar"),
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
    fn environment_parses_and_templates() {
        let config: EnvironmentConfig =
            serde_yaml::from_str(TEMPLATED_ENVIRONMENT).expect("environment config to parse");

        let mut res = config.required_values();
        res.sort(); // Sorting so values are in a determistic order for the assert_eq
        assert_eq!(res, &["bar", "foo"], "expected values to match")
    }

    // Tests

    #[dir_cases("crates/rtf-config/resources/config-tests/environment/check-failures")]
    #[test]
    fn check_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "check-errors");

        let res: serde_yaml::Result<EnvironmentConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let dir = PathBuf::from("resources/config-tests/environment/check-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir);

        let env_config = res.unwrap();
        let res = env_config.try_check(&mut vec!["environment".to_string()], &src, &ctx);

        assert!(res.is_err(), "expected check failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds: Vec<String> = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
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
        let environment =
            template_trait_test_config(setup_field, r("setup2"), teardown_field, r("teardown2"));

        let res = environment.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(p("setup1"), p("setup2"), p("teardown1"), p("teardown2"), &["setup1", "setup2", "teardown1", "teardown2"]; "both setup and both teardown pending requires values")]
    #[test_case(p("setup1"), p("setup2"), p("teardown1"), r("teardown2"), &["setup1", "setup2", "teardown1"]; "both setup and single teardown pending requires values")]
    #[test_case(p("setup1"), p("setup2"), r("teardown1"), r("teardown2"), &["setup1", "setup2"]; "both setup and no teardown pending requires values")]
    #[test_case(p("setup1"), r("setup2"), p("teardown1"), p("teardown2"), &["setup1", "teardown1", "teardown2"]; "single setup and both teardown pending requires values")]
    #[test_case(p("setup1"), r("setup2"), p("teardown1"), r("teardown2"), &["setup1", "teardown1"]; "single setup and single teardown pending requires values")]
    #[test_case(p("setup1"), r("setup2"), r("teardown1"), r("teardown2"), &["setup1"]; "single setup and no teardown pending requires values")]
    #[test_case(r("setup1"), r("setup2"), p("teardown1"), p("teardown2"), &["teardown1", "teardown2"]; "no setup and both teardown pending requires values")]
    #[test_case(r("setup1"), r("setup2"), p("teardown1"), r("teardown2"), &["teardown1"]; "no setup and single teardown pending requires values")]
    #[test_case(r("setup1"), r("setup2"), r("teardown1"), r("teardown2"), &[]; "no setup and no teardown pending requires no values")]
    #[test]
    fn required_values(
        setup_field_1: Field<String>,
        setup_field_2: Field<String>,
        teardown_field_1: Field<String>,
        teardown_field_2: Field<String>,
        expected: &[&str],
    ) {
        let environment = template_trait_test_config(
            setup_field_1,
            setup_field_2,
            teardown_field_1,
            teardown_field_2,
        );

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

        let values = value_map(field_names.as_slice());
        let mut environment =
            test_environment_config(field_names.as_slice(), setup_fields, teardown_fields);

        let res = environment.try_template(&mut Vec::new(), &values);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    /// Helper function for asserting template errors are as expected
    fn assert_template_errors(
        mut environment: EnvironmentConfig,
        values: HashMap<String, Scalar>,
        expected_setup_err_fields: &[&str],
        expected_teardown_err_fields: &[&str],
    ) {
        let (mut expected_err_messages, mut expected_err_paths) =
            expected_error_details(expected_setup_err_fields, "setup");
        let (expected_messages, expected_paths) =
            expected_error_details(expected_teardown_err_fields, "teardown");
        expected_err_messages.extend(expected_messages);
        expected_err_paths.extend(expected_paths);

        let res = environment.try_template(&mut Vec::new(), &values);
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
        let values = value_map(&["setup", "setup1", "setup2"]);
        let environment = test_environment_config(value_defs, setup_fields, &[]);

        assert_template_errors(environment, values, expected_err_fields, &[]);
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
        let values = value_map(&["teardown", "teardown1", "teardown2"]);
        let environment = test_environment_config(value_defs, &[], teardown_fields);

        assert_template_errors(environment, values, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_value_definitions() {
        let values = value_map(&["setup", "teardown"]);
        let environment = test_environment_config(&[], &["setup"], &["teardown"]);

        assert_template_errors(environment, values, &["setup"], &["teardown"]);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_values_not_provided() {
        let values = value_map(&[]);
        let environment =
            test_environment_config(&["setup", "teardown"], &["setup"], &["teardown"]);

        assert_template_errors(environment, values, &["setup"], &["teardown"]);
    }

    /// Tests that setup cannot access values from setup.provides.
    /// Setup can only access values declared in the top-level `values` section.
    #[test]
    fn try_template_setup_cannot_access_provides_values() {
        // Setup a config where values are only defined in setup.provides, not in top-level values
        let mut config = scoping_test_config(&[], &["foo", "setup-path"]);

        // Values exist in the map but are only defined in provides
        let values = value_map(&["foo", "setup-path"]);

        let res = config.try_template_setup(&mut Vec::new(), &values);

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
        let mut config = scoping_test_config(&[], &["bar", "teardown-path"]);

        // Values exist in the map and are defined in provides
        let values = value_map(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &values);

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
        let mut config = scoping_test_config(&["foo", "setup-path"], &[]);

        // Values exist in the map and are defined in top-level values
        let values: HashMap<String, Scalar> = [("foo", "a"), ("setup-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let res = config.try_template_setup(&mut Vec::new(), &values);

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
        let mut config = scoping_test_config(&["bar", "teardown-path"], &[]);

        // Values exist in the map and are defined in top-level values
        let values = value_map(&["bar", "teardown-path"]);

        let res = config.try_template_teardown(&mut Vec::new(), &values);

        // Teardown should succeed because it can access top-level values
        assert!(
            res.is_ok(),
            "expected teardown to succeed when accessing top-level values, got {res:?}"
        );
    }
}
