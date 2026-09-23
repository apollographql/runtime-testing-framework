use crate::{
    VariableDefinition,
    context::ResolutionContext,
    inlining::{self, Inline, InlineMode, InlinedProvider},
    providers::{
        self,
        command::CommandSection,
        file::{RawSource, SourceDir, StableSource},
    },
    templating::{self, Scalar, Template, TemplateContext},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
};

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
    #[serde(default)]
    pub variable_definitions: Vec<VariableDefinition>,
    /// The command to execute as this custom provider
    #[serde(flatten)]
    pub command: CommandSection,
}

impl CustomProviderDefinition {
    /// Validate variable definitions and provided values against allowed_values constraints.
    pub fn validate_variables(
        &self,
        variables: &HashMap<String, Scalar>,
        cli_overrides: Option<&HashMap<String, StableSource>>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        for vd in &self.variable_definitions {
            vd.validate(
                &["variable_definitions".to_string(), vd.name.to_string()],
                &mut errs,
            );

            if let Some(value) = variables.get(&vd.name) {
                let source_desc = match cli_overrides {
                    None => "test plan variable",
                    Some(overrides) => match overrides.get(&vd.name) {
                        Some(StableSource::Cli | StableSource::VariablesFile) => "CLI variable",
                        _ => "test plan variable",
                    },
                };
                vd.validate_value(
                    value,
                    source_desc,
                    &["variables".to_string(), vd.name.to_string()],
                    &mut errs,
                );
            }
        }

        errs.into_result(())
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
    fn required_variables(&self) -> Vec<String> {
        self.command.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let file_ctx = ctx.for_config_file(file_source, None, self.variable_definitions.iter());

        self.command
            .validate_context(path, allowed_variables, file_source, &file_ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let file_ctx = ctx.for_config_file(source, None, self.variable_definitions.iter());

        self.command
            .try_template_nested(path, "command_section", source, &file_ctx)
    }
}

impl Inline for CustomProviderDefinition {
    fn try_inline<'a>(
        &'a mut self,
        mode: InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        self.command.try_inline(mode, ctx, cache)
    }
}

/// # Custom Provider Declaration
///
/// Allows for declaring one or more custom providers that should be loaded from the given
/// directory. The directory can be specified as a relative path from the file containing
/// this declaration or as coming from a GitHub repository.
///
/// ## Example using a relative directory
///
/// ```yaml
/// kind: local
/// relative_path: ../providers
/// using:
///   my_custom_provider: my_custom_provider.yaml
///   my_other_provider: my_other_provider.yaml
/// ```
///
/// ## Example using a directory from a GitHub repository
///
/// ```yaml
/// kind: github
/// org: my-org
/// repo: my-repo
/// path: path/to/providers
/// git_ref: main
/// using:
///   my_custom_provider: my_custom_provider.yaml
///   my_other_provider: my_other_provider.yaml
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct CustomProviderDeclaration {
    /// The source directory containing the custom provider definition files
    #[serde(flatten)]
    pub source: RawSource,

    /// A map of the name to use for a given custom provider to the child path within this
    /// directory containing the definition for that provider.
    pub using: HashMap<String, String>,
}

impl CustomProviderDeclaration {
    pub async fn try_load_all(
        &self,
        file_source: &SourceDir,
        ctx: &impl ResolutionContext,
    ) -> Result<
        HashMap<String, (SourceDir, CustomProviderDefinition)>,
        Vec<(String, providers::Error)>,
    > {
        let mut providers = HashMap::with_capacity(self.using.len());
        let mut errs = Vec::new();

        for (provider_name, filename) in self.using.iter() {
            match load_one(self.source.with_child_path(filename), file_source, ctx).await {
                Ok(entry) => {
                    providers.insert(provider_name.clone(), entry);
                }
                Err(e) => errs.push((provider_name.clone(), e)),
            }
        }

        if errs.is_empty() {
            Ok(providers)
        } else {
            errs.sort_by(|l, r| l.0.cmp(&r.0));
            Err(errs)
        }
    }
}

async fn load_one(
    source: RawSource,
    file_source: &SourceDir,
    ctx: &impl ResolutionContext,
) -> providers::Result<(SourceDir, CustomProviderDefinition)> {
    let (definition_source, file_name) = source.try_into_source_and_filename(file_source, ctx)?;
    let content = definition_source
        .try_get_file_content(file_name, ctx)
        .await?;
    let raw: serde_yaml::Value = serde_yaml::from_str(&content)?;

    if let Some(mapping) = raw.as_mapping()
        && mapping.contains_key("custom_providers")
    {
        return Err(providers::Error::NestedCustomProvider);
    }

    Ok((definition_source, serde_yaml::from_value(raw)?))
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
        VariableDefinition,
        context::Context,
        formats::{
            custom_provider::test_helpers::{
                custom_provider_with_fields, templatable_custom_provider,
            },
            providers::test_helpers::create_temp_dir_with_file,
            tests::{assert_template_errors, expected_error_details, p, r, template_context},
        },
        mock_context::MockContext,
        templating::{ErrorKind, Field},
    };
    use assert_fs::{
        fixture::PathChild,
        prelude::{FileWriteStr, PathCreateDir},
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
        res.sort(); // Sorting so variables are in a deterministic order for the assert_eq
        assert_eq!(res, &["bar", "foo"], "expected variables to match")
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

        let res = config.try_template(&mut Vec::new(), &StableSource::TestPlan, &variables);
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

    const RELATIVE_DIR_DECLARATION: &str = indoc!(
        r#"
        kind: local
        relative_path: ../providers
        using:
          my_custom_provider: my_custom_provider.yaml
        "#
    );

    const GITHUB_DIR_DECLARATION: &str = indoc!(
        r#"
        kind: github
        org: my-org
        repo: my-repo
        path: path/to/providers
        git_ref: main
        using:
          my_custom_provider: my_custom_provider.yaml
        "#
    );

    const GITHUB_DIR_DECLARATION_NO_REF: &str = indoc!(
        r#"
        kind: github
        org: my-org
        repo: my-repo
        path: path/to/providers
        using:
          my_custom_provider: my_custom_provider.yaml
        "#
    );

    #[test_case(RELATIVE_DIR_DECLARATION; "relative dir")]
    #[test_case(GITHUB_DIR_DECLARATION; "github dir")]
    #[test_case(GITHUB_DIR_DECLARATION_NO_REF; "github dir without ref")]
    #[test]
    fn parse_declaration(raw: &str) {
        assert!(serde_yaml::from_str::<'_, CustomProviderDeclaration>(raw).is_ok());
    }

    #[tokio::test]
    async fn declaration_try_load_all_local_success() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        let providers = temp.child("providers");
        providers.create_dir_all().unwrap();
        for fname in ["my_provider.yaml", "my_other_provider.yaml"] {
            providers
                .child(fname)
                .write_str(TEMPLATED_CUSTOM_PROVIDER)
                .unwrap();
        }

        let declaration = CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: "providers".into(),
            },
            using: [
                ("my_provider".into(), "my_provider.yaml".into()),
                ("my_other_provider".into(), "my_other_provider.yaml".into()),
            ]
            .into_iter()
            .collect(),
        };

        let definitions = declaration
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await
            .unwrap();

        assert_eq!(
            &definitions.get("my_provider").unwrap().0,
            &SourceDir::local(providers.canonicalize().unwrap())
        );

        assert_eq!(
            &definitions.get("my_other_provider").unwrap().0,
            &SourceDir::local(providers.canonicalize().unwrap())
        );
    }

    #[tokio::test]
    async fn declaration_try_load_all_github_success() {
        let ctx = MockContext::with_github_client(&[
            (
                "my-org/my-repo/providers/my_provider.yaml",
                TEMPLATED_CUSTOM_PROVIDER,
            ),
            (
                "my-org/my-repo/providers/my_other_provider.yaml",
                TEMPLATED_CUSTOM_PROVIDER,
            ),
        ]);

        let declaration = CustomProviderDeclaration {
            source: RawSource::Github {
                org: "my-org".to_string(),
                repo: "my-repo".to_string(),
                path: "providers".into(),
                git_ref: None,
            },
            using: [
                ("my_provider".into(), "my_provider.yaml".into()),
                ("my_other_provider".into(), "my_other_provider.yaml".into()),
            ]
            .into_iter()
            .collect(),
        };

        let definitions = declaration
            .try_load_all(&SourceDir::local("/config"), &ctx)
            .await
            .unwrap();

        let no_ref: Option<&str> = None;

        assert_eq!(
            &definitions.get("my_provider").unwrap().0,
            &SourceDir::github("my-org", "my-repo", "providers", no_ref),
        );

        assert_eq!(
            &definitions.get("my_other_provider").unwrap().0,
            &SourceDir::github("my-org", "my-repo", "providers", no_ref),
        );
    }

    // Helper functions for validate_variables tests
    fn vd_with_allowed(
        name: &str,
        allowed: Option<&[&str]>,
        default: Option<&str>,
    ) -> VariableDefinition {
        VariableDefinition {
            name: name.into(),
            description: format!("description for {name}"),
            default: default.map(|s| s.into()),
            allowed_values: allowed.map(|a| a.iter().map(|&s| s.into()).collect()),
        }
    }

    fn def_with_variable_definitions(vds: Vec<VariableDefinition>) -> CustomProviderDefinition {
        CustomProviderDefinition {
            variable_definitions: vds,
            ..CustomProviderDefinition::empty()
        }
    }

    fn scalar_vars(pairs: &[(&str, &str)]) -> HashMap<String, crate::templating::Scalar> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), (*v).into()))
            .collect()
    }

    fn source_overrides(names: &[&str]) -> HashMap<String, StableSource> {
        names
            .iter()
            .map(|&n| (n.to_string(), StableSource::Cli))
            .collect()
    }

    #[test]
    fn validate_variables_empty_allowed_values_is_detected() {
        let def = def_with_variable_definitions(vec![vd_with_allowed("foo", Some(&[]), None)]);
        let result = def.validate_variables(&HashMap::new(), None);

        let err = result.unwrap_err().unwrap_single();
        assert!(matches!(err.kind, ErrorKind::EmptyAllowedValues));
        assert!(err.message.contains("foo"));
    }

    #[test]
    fn validate_variables_default_not_in_allowed_values_is_detected() {
        let def = def_with_variable_definitions(vec![vd_with_allowed(
            "foo",
            Some(&["a", "b"]),
            Some("c"),
        )]);
        let result = def.validate_variables(&HashMap::new(), None);

        let err = result.unwrap_err().unwrap_single();
        assert!(matches!(err.kind, ErrorKind::DefaultNotInAllowedValues));
        assert!(err.message.contains("foo"));
        assert!(err.message.contains("c"));
    }

    #[test]
    fn validate_variables_value_not_in_allowed_with_cli_overrides_none_says_test_variable() {
        let def =
            def_with_variable_definitions(vec![vd_with_allowed("foo", Some(&["a", "b"]), None)]);
        let variables = scalar_vars(&[("foo", "invalid")]);

        let result = def.validate_variables(&variables, None);

        let err = result.unwrap_err().unwrap_single();
        assert!(matches!(err.kind, ErrorKind::ValueNotAllowed));
        assert!(err.message.contains("test plan variable"));
    }

    #[test]
    fn validate_variables_value_not_in_allowed_with_key_in_cli_overrides_says_cli_variable() {
        let def =
            def_with_variable_definitions(vec![vd_with_allowed("foo", Some(&["a", "b"]), None)]);
        let variables = scalar_vars(&[("foo", "invalid")]);
        let cli = source_overrides(&["foo"]);

        let result = def.validate_variables(&variables, Some(&cli));

        let err = result.unwrap_err().unwrap_single();
        assert!(matches!(err.kind, ErrorKind::ValueNotAllowed));
        assert!(err.message.contains("CLI variable"));
    }

    #[test]
    fn validate_variables_value_not_in_allowed_with_key_not_in_cli_overrides_says_variable() {
        let def =
            def_with_variable_definitions(vec![vd_with_allowed("foo", Some(&["a", "b"]), None)]);
        let variables = scalar_vars(&[("foo", "invalid")]);
        let cli = source_overrides(&["other"]); // foo not in cli overrides

        let result = def.validate_variables(&variables, Some(&cli));

        let err = result.unwrap_err().unwrap_single();
        assert!(matches!(err.kind, ErrorKind::ValueNotAllowed));
        // Should say "test plan variable" not "CLI variable"
        assert!(err.message.contains("test plan variable"));
        assert!(!err.message.contains("CLI"));
    }

    #[test]
    fn validate_variables_valid_values_pass_through() {
        let def =
            def_with_variable_definitions(vec![vd_with_allowed("foo", Some(&["a", "b"]), None)]);
        let variables = scalar_vars(&[("foo", "a")]);

        let result = def.validate_variables(&variables, None);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_variables_unconstrained_variables_always_pass() {
        let def = def_with_variable_definitions(vec![vd_with_allowed("foo", None, None)]);
        let variables = scalar_vars(&[("foo", "anything")]);

        let result = def.validate_variables(&variables, None);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_variables_multiple_errors_are_collected() {
        let def = def_with_variable_definitions(vec![
            vd_with_allowed("foo", Some(&[]), None), // empty allowed_values
            vd_with_allowed("bar", Some(&["a"]), Some("b")), // default not in allowed
        ]);

        let result = def.validate_variables(&HashMap::new(), None);

        let errors = result.unwrap_err().into_vec();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn validate_variables_without_definition_are_ignored() {
        // Variables present in the map but not in variable_definitions should not cause errors
        let def =
            def_with_variable_definitions(vec![vd_with_allowed("foo", Some(&["a", "b"]), None)]);
        let variables = scalar_vars(&[("foo", "a"), ("extra", "value")]);

        let result = def.validate_variables(&variables, None);
        assert!(result.is_ok());
    }
}
