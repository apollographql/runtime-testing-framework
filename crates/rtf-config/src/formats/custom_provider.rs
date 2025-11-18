use crate::{
    VariableDefinition,
    formats::Result,
    providers::command::CommandSection,
    providers::file::Source,
    templating::{self, Template, TemplateContext},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// # Custom Provider Definition
///
/// The definition for a custom provider to be executed as part of a test plan.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct CustomProviderDefinition {
    /// The name of this custom provider
    pub name: String,
    /// A brief description of the purpose / behaviour of this custom provider
    pub description: String,
    /// The variables that are required for this custom provider
    pub variable_definitions: Vec<VariableDefinition>,
    /// The command to execute as this custom provider
    #[serde(flatten)]
    pub command: CommandSection,
}

impl CustomProviderDefinition {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    /// Create an empty [CustomProviderDefinition] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> CustomProviderDefinition {
        CustomProviderDefinition {
            name: Default::default(),
            description: Default::default(),
            variable_definitions: Default::default(),
            command: CommandSection::empty(),
        }
    }
}

// The Template implementation for CustomProviderDefinition is used to check that the custom provider definition is self-consistent
// i.e. the template variables it uses are defined in the variable_definitions field. It does not check that the user has provided
// all the required variables for the custom provider in the NamedFileProviders for the config file it is being used in.
// Check is intentionally not implemented for CustomProviderDefinition as there is no way to guarantee it will be in its fully templated
// form when a check is performed. It is only possible to check a custom provider definition after it has been transformed into a
// FromCommand Provider.
impl Template for CustomProviderDefinition {
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
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let allowed_variables = ctx.for_config_file(source, self.variable_definitions.iter());

        self.command
            .try_template_nested(path, "command_section", source, &allowed_variables)
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

    /// Create a CustomProviderConfig for testing Template trait methods (has_pending_fields, required_variables)
    pub(crate) fn custom_provider_with_fields(
        fields: &[Field<String>],
    ) -> CustomProviderDefinition {
        CustomProviderDefinition {
            command: CommandSection {
                file_providers: named_file_providers_with_fields(fields),
                ..CommandSection::empty()
            },
            ..CustomProviderDefinition::empty()
        }
    }

    /// Create a CustomProviderDefinition for template testing
    pub(crate) fn templatable_custom_provider(
        variable_names: &[&str],
        custom_provider_fields: &[&str],
    ) -> CustomProviderDefinition {
        CustomProviderDefinition {
            variable_definitions: variable_definitions(variable_names),
            command: CommandSection {
                file_providers: templatable_file_providers(custom_provider_fields),
                ..CommandSection::empty()
            },
            ..CustomProviderDefinition::empty()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        formats::{
            custom_provider::test_helpers::{
                custom_provider_with_fields, templatable_custom_provider,
            },
            tests::{assert_template_errors, expected_error_details, p, r, template_context},
        },
        templating::Field,
    };
    use indoc::indoc;
    use serde_yaml;
    use simple_test_case::test_case;

    // An example custom provider config to check parsing and templating
    const TEMPLATED_CUSTOM_PROVIDER: &str = indoc!(
        r#"
        name: custom provider
        description: A templated custom provider
        variable_definitions:
          - name: foo
            description: a value foo
          - name: bar
            description: a value bar
        command:
          name: custom_provider.sh
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
        let config: CustomProviderDefinition = serde_yaml::from_str(TEMPLATED_CUSTOM_PROVIDER)
            .expect("custom provider config to parse");

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a determistic order for the assert_eq
        assert_eq!(res, &["bar", "foo"], "expected variables to match")
    }

    #[test_case(&[p("foo")], true; "single field is pending")]
    #[test_case(&[r("foo")], false; "single field is resolved")]
    #[test_case(&[p("foo"), p("bar")], true; "multiple fields pending is pending")]
    #[test_case(&[p("foo"), r("bar")], true; "multiple fields with single field pending is pending")]
    #[test_case(&[r("foo"), r("bar")], false; "multiple fields none pending is resolved")]
    #[test]
    fn has_pending_fields(fields: &[Field<String>], expected: bool) {
        let config = custom_provider_with_fields(fields);
        assert_eq!(
            config.has_pending_fields(),
            expected,
            "expected has_pending_fields to be {expected}"
        )
    }

    #[test_case(&[p("foo")], &["foo"]; "single field is required")]
    #[test_case(&[r("foo")], &[]; "single field resolved requires no variables")]
    #[test_case(&[p("field1"), p("field2")], &["field1", "field2"]; "multiple fields pending requires variables")]
    #[test_case(&[p("field1"), r("field2")], &["field1"]; "multiple fields with single pending requires variables")]
    #[test_case(&[r("field1"), r("field2")], &[]; "multiple fields none pending requires no variables")]
    #[test]
    fn required_variables(fields: &[Field<String>], expected: &[&str]) {
        let config = custom_provider_with_fields(fields);

        let res = config.required_variables();
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
        let mut config = templatable_custom_provider(field_names, field_names);
        let variables = template_context(field_names);

        let res = config.try_template(&mut Vec::new(), &Source::local("/"), &variables);
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
        custom_provider_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let variables = template_context(variables);
        let mut config = templatable_custom_provider(variable_defs, custom_provider_fields);

        let (expected_err_messages, expected_err_paths) =
            expected_error_details(expected_err_fields, "command_section");

        assert_template_errors(
            &mut config,
            variables,
            expected_err_messages,
            expected_err_paths,
        );
    }
}
