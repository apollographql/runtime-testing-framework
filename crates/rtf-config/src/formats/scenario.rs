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
pub(crate) mod test_helpers {
    use super::*;
    use crate::{
        formats::tests::{
            named_file_providers_with_fields, templatable_file_providers, value_definitions,
        },
        templating::Field,
    };

    /// Create a ScenarioConfig for testing Template trait methods (has_pending_fields, required_values)
    pub(crate) fn scenario_with_fields(fields: &[Field<String>]) -> ScenarioConfig {
        ScenarioConfig {
            command: CommandSection {
                file_providers: named_file_providers_with_fields(fields),
                ..CommandSection::empty()
            },
            ..ScenarioConfig::empty()
        }
    }

    /// Create a ScenarioConfig for template testing
    pub(crate) fn templatable_scenario(
        value_names: &[&str],
        scenario_fields: &[&str],
    ) -> ScenarioConfig {
        ScenarioConfig {
            values: value_definitions(value_names),
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
        formats::scenario::test_helpers::{scenario_with_fields, templatable_scenario},
        formats::tests::{assert_template_errors, expected_error_details, p, r, value_map},
        templating::Field,
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

    #[test_case(&[p("foo")], true; "single field is pending")]
    #[test_case(&[r("foo")], false; "single field is resolved")]
    #[test_case(&[p("field1"), p("field2")], true; "multiple fields pending is pending")]
    #[test_case(&[p("field1"), r("field2")], true; "multiple fields with single field pending is pending")]
    #[test_case(&[r("field1"), r("field2")], false; "multiple fields none pending is resolved")]
    #[test]
    fn has_pending_fields(fields: &[Field<String>], expected: bool) {
        let scenario = scenario_with_fields(fields);

        let res = scenario.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_fields has expected value"
        )
    }

    #[test_case(&[p("foo")], &["foo"]; "single field is required")]
    #[test_case(&[r("foo")], &[]; "single field resolved requires no values")]
    #[test_case(&[p("field1"), p("field2")], &["field1", "field2"]; "multiple fields pending requires values")]
    #[test_case(&[p("field1"), r("field2")], &["field1"]; "multiple fields with single pending requires values")]
    #[test_case(&[r("field1"), r("field2")], &[]; "multiple fields none pending requires no values")]
    #[test]
    fn required_values(fields: &[Field<String>], expected: &[&str]) {
        let scenario = scenario_with_fields(fields);

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
        let mut scenario = templatable_scenario(field_names, field_names);

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
        let mut scenario = templatable_scenario(value_defs, scenario_fields);

        let (expected_err_messages, expected_err_paths) =
            expected_error_details(expected_err_fields, "command_section");

        assert_template_errors(
            &mut scenario,
            values,
            expected_err_messages,
            expected_err_paths,
        );
    }
}
