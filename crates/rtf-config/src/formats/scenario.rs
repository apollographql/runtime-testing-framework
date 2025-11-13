use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    formats::Result,
    providers::{command::CommandSection, file::Source},
    templating::{self, Template, TemplateVariables},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

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

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &Source,
        variables: &TemplateVariables,
    ) -> templating::Result<()> {
        let allowed_variables = variables.for_config_file(source, self.variable_definitions.iter());

        self.command
            .try_template_nested(path, "command_section", source, &allowed_variables)
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
        variable_names: &[&str],
        scenario_fields: &[&str],
    ) -> ScenarioConfig {
        ScenarioConfig {
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
                template_variables,
            },
        },
        providers::command::test_helpers::{cmd_with_inline_file, cmd_with_required_file},
        templating::Field,
    };
    use indoc::indoc;
    use simple_test_case::test_case;

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
    fn parse_and_template() {
        let config: ScenarioConfig =
            serde_yaml::from_str(TEMPLATED_SCENARIO).expect("scenario config to parse");

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a determistic order for the assert_eq
        assert_eq!(res, &["bar", "foo"], "expected variables to match")
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
    #[test_case(&[r("foo")], &[]; "single field resolved requires no variables")]
    #[test_case(&[p("field1"), p("field2")], &["field1", "field2"]; "multiple fields pending requires variables")]
    #[test_case(&[p("field1"), r("field2")], &["field1"]; "multiple fields with single pending requires variables")]
    #[test_case(&[r("field1"), r("field2")], &[]; "multiple fields none pending requires no variables")]
    #[test]
    fn required_variables(fields: &[Field<String>], expected: &[&str]) {
        let scenario = scenario_with_fields(fields);

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
        let variables = template_variables(field_names);
        let mut scenario = templatable_scenario(field_names, field_names);

        let res = scenario.try_template(&mut Vec::new(), &Source::local("/"), &variables);
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
        let variables = template_variables(variables);
        let mut scenario = templatable_scenario(variable_defs, scenario_fields);

        let (expected_err_messages, expected_err_paths) =
            expected_error_details(expected_err_fields, "command_section");

        assert_template_errors(
            &mut scenario,
            variables,
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
}
