use crate::{
    ValueDefinition,
    checks::{self, Check},
    context::ResolutionContext,
    formats::{Result, values_for_config_file},
    providers::{command::CommandSection, file::Source},
    templating::{self, Scalar, Template},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

/// # Scenario Config
///
/// Configuration for a single test scenario to be executed as part of a test plan.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ScenarioConfig {
    /// The name of this scenario
    pub name: String,
    /// A brief description of the purpose / behaviour of this scenario
    pub description: String,
    /// Definitions for the required values for templating this scenario
    #[serde(default)]
    pub values: Vec<ValueDefinition>,
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
            values: Default::default(),
            command: CommandSection::empty(),
        }
    }
}

impl Template for ScenarioConfig {
    fn has_pending_fields(&self) -> bool {
        self.command.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        self.command.required_values()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let definitions = self.values.iter();
        let allowed_values = values_for_config_file(values, definitions);

        self.command
            .try_template_nested(path, "command_section", &allowed_values)
    }
}

impl Check for ScenarioConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        self.command.try_check_nested(path, "command", src, ctx)
    }
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

    /// Create a ScenarioConfig for testing Template trait methods (has_pending_fields, required_values)
    fn template_trait_test_config(field: Field<String>) -> ScenarioConfig {
        ScenarioConfig {
            command: CommandSection {
                file_providers: vec![named_file_provider_with_field("test", field)],
                ..CommandSection::empty()
            },
            ..ScenarioConfig::empty()
        }
    }

    /// Create a ScenarioConfig for testing Template trait methods with multiple fields
    fn template_trait_test_config_multi(
        field_1: Field<String>,
        field_2: Field<String>,
    ) -> ScenarioConfig {
        ScenarioConfig {
            command: CommandSection {
                file_providers: vec![
                    named_file_provider_with_field("test1", field_1),
                    named_file_provider_with_field("test2", field_2),
                ],
                ..CommandSection::empty()
            },
            ..ScenarioConfig::empty()
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

    /// Create a test ScenarioConfig with specified field names
    /// All field names are added as both value definitions and pending template fields
    fn test_scenario_config(value_names: &[&str], scenario_fields: &[&str]) -> ScenarioConfig {
        ScenarioConfig {
            values: value_definitions(value_names),
            command: CommandSection {
                file_providers: file_providers_from_names(scenario_fields),
                ..CommandSection::empty()
            },
            ..ScenarioConfig::empty()
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

    // An example scenario config to check parsing and templating
    const TEMPLATED_SCENARIO: &str = indoc!(
        r#"
        name: scenario
        description: a templated scenario
        values:
          - name: foo
            description: a value foo
          - name: bar
            description: a value bar
            default: "bar"
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

    #[test]
    fn scenario_parses_and_templates() {
        let config: ScenarioConfig =
            serde_yaml::from_str(TEMPLATED_SCENARIO).expect("scenario config to parse");

        let mut res = config.required_values();
        res.sort(); // Sorting so values are in a determistic order for the assert_eq
        assert_eq!(res, &["bar", "foo"], "expected values to match")
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/scenario/check-failures")]
    #[test]
    fn check_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "check-errors");

        let res: serde_yaml::Result<ScenarioConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let dir = PathBuf::from("resources/config-tests/scenario/check-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir);

        let scenario_config = res.unwrap();
        let res = scenario_config.try_check(&mut Vec::new(), &src, &ctx);

        assert!(res.is_err(), "expected check failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
        );
    }

    // Tests

    #[test_case(p("foo"), true; "single field is pending")]
    #[test_case(r("foo"), false; "single field is resolved")]
    #[test]
    fn has_pending_fields(field: Field<String>, expected: bool) {
        let scenario = template_trait_test_config(field);

        let res = scenario.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(p("field1"), p("field2"), true; "both fields pending is pending")]
    #[test_case(p("field1"), r("field2"), true; "single field pending is pending")]
    #[test_case(r("field1"), r("field2"), false; "no fields pending is resolved")]
    #[test]
    fn has_pending_fields_multi(field_1: Field<String>, field_2: Field<String>, expected: bool) {
        let scenario = template_trait_test_config_multi(field_1, field_2);

        let res = scenario.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(p("foo"), &["foo"]; "single field is required")]
    #[test_case(r("foo"), &[]; "single field resolved requires no values")]
    #[test]
    fn required_values(field: Field<String>, expected: &[&str]) {
        let scenario = template_trait_test_config(field);

        let res = scenario.required_values();
        assert_eq!(
            res, expected,
            "tests that required_values has expected value"
        )
    }

    #[test_case(p("field1"), p("field2"), &["field1", "field2"]; "both fields pending requires values")]
    #[test_case(p("field1"), r("field2"), &["field1"]; "single field pending requires values")]
    #[test_case(r("field1"), r("field2"), &[]; "no fields pending requires no values")]
    #[test]
    fn required_values_multi(field_1: Field<String>, field_2: Field<String>, expected: &[&str]) {
        let scenario = template_trait_test_config_multi(field_1, field_2);

        let res = scenario.required_values();
        assert_eq!(
            res, expected,
            "tests that required_values has expected value"
        )
    }

    #[test_case(&["foo"]; "single value")]
    #[test_case(&["foo", "bar"]; "multiple values")]
    #[test_case(&["foo", "bar", "baz"]; "three values")]
    #[test_case(&[]; "no values")]
    #[test]
    fn try_template_succeeds(field_names: &[&str]) {
        let values = value_map(field_names);
        let mut scenario = test_scenario_config(field_names, field_names);

        let res = scenario.try_template(&mut Vec::new(), &values);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test_case(&["foo"], &[], &["foo"], &["foo"]; "single value provided and not defined")]
    #[test_case(&[], &["foo", "bar", "baz"], &["foo", "bar", "baz"], &["bar", "baz", "foo"]; "no values provided and multiple defined")]
    #[test_case(&["foo", "bar", "baz"], &["foo", "bar"], &["foo", "bar", "baz"], &["baz"]; "multiple values provided and one not defined")]
    #[test_case(&[], &["foo"], &["foo"], &["foo"]; "no values provided but single value defined")]
    #[test_case(&[], &["foo", "bar", "baz"], &["foo", "bar", "baz"], &["bar", "baz", "foo"]; "no values provided but multiple values defined")]
    #[test_case(&["foo", "bar"], &["foo", "bar", "baz"], &["foo", "bar", "baz"], &["baz"]; "one provided value missing when multiple values defined")]
    #[test]
    fn try_template_missing_value_definitions(
        values: &[&str],
        value_defs: &[&str],
        scenario_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let values = value_map(values);
        let mut scenario = test_scenario_config(value_defs, scenario_fields);

        let (expected_err_messages, expected_err_paths) =
            expected_error_details(expected_err_fields, "command_section");

        let res = scenario.try_template(&mut Vec::new(), &values);
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
}
