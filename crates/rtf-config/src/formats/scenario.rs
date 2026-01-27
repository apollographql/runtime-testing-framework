use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    formats::{CustomProviderDeclaration, Result},
    inlining,
    providers::{
        self, Provider,
        command::CommandSection,
        file::{NamedFileProvider, SourceDir},
    },
    run::{Execute, OUTDIR, OUTPUT_PATH, RunProviders},
    templating::{self, Field, FileType, Scalar, Template, TemplateContext},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    pin::Pin,
};

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
    pub command: ScenarioCommand,
}

impl ScenarioConfig {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    pub async fn inline(&mut self, ctx: &impl ResolutionContext) -> inlining::Result<()> {
        self.command.inline(ctx).await
    }

    pub async fn inline_all_relative_paths(
        &mut self,
        ctx: &impl ResolutionContext,
    ) -> inlining::Result<()> {
        self.command.inline_all_relative_paths(ctx).await
    }

    /// Create an empty [ScenarioConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> ScenarioConfig {
        ScenarioConfig {
            name: Default::default(),
            description: Default::default(),
            variable_definitions: Default::default(),
            custom_providers: Default::default(),
            command: ScenarioCommand::Script(CommandSection::empty()),
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
        file_source: &SourceDir,
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
        source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let ctx = ctx.for_config_file(
            source,
            Some(FileType::Scenario),
            self.variable_definitions.iter(),
        );

        let mut path = path.clone();
        if path.last().map(String::as_str) != Some("scenario") {
            path.push("scenario".to_string());
        }
        self.command.try_template(&mut path, source, &ctx)
    }
}

impl Check for ScenarioConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        if path.last().map(String::as_str) != Some("scenario") {
            path.push("scenario".to_string());
        }

        match &self.command {
            ScenarioCommand::Docker(inner) => inner.try_check(path, ctx),
            ScenarioCommand::Script(inner) => inner.try_check(path, ctx),
        }
    }
}

impl CheckArrayDuplicates for ScenarioConfig {
    const BASE_PATH: &str = "scenario";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        let file_providers = match &mut self.command {
            ScenarioCommand::Docker(inner) => &mut inner.file_providers,
            ScenarioCommand::Script(inner) => &mut inner.file_providers,
        };

        vec![
            (
                "variables",
                DedupArray::VariableDef(&mut self.variable_definitions),
            ),
            ("file_providers", DedupArray::Nfp(file_providers)),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(untagged)]
pub enum ScenarioCommand {
    Docker(DockerScenario),
    Script(CommandSection),
}

impl RunProviders for ScenarioCommand {
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        match self {
            Self::Docker(inner) => inner.file_providers.run_providers(providers_dir, ctx).await,
            Self::Script(inner) => inner.run_providers(providers_dir, ctx).await,
        }
    }

    fn inline<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        match self {
            Self::Docker(inner) => inner.file_providers.inline(ctx),
            Self::Script(inner) => inner.inline(ctx),
        }
    }

    fn inline_all_relative_paths<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        match self {
            Self::Docker(inner) => inner.file_providers.inline_all_relative_paths(ctx),
            Self::Script(inner) => inner.inline_all_relative_paths(ctx),
        }
    }
}

impl Execute for ScenarioCommand {
    fn command_name(&self) -> &str {
        match self {
            Self::Docker(inner) => inner.command_name(),
            Self::Script(inner) => inner.command_name(),
        }
    }

    fn execute(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        match self {
            Self::Docker(inner) => inner.execute(out_dir, output_path, ctx),
            Self::Script(inner) => inner.execute(out_dir, output_path, ctx),
        }
    }
}

impl Check for ScenarioCommand {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        match self {
            Self::Docker(inner) => inner.try_check(path, ctx),
            Self::Script(inner) => inner.try_check(path, ctx),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct DockerScenario {
    pub image: Field<String>,
    pub tag: Option<Field<String>>,
    pub command: Field<String>,

    /// Environment variables to set
    #[serde(default)]
    pub env_vars: HashMap<String, Field<Scalar>>,
    /// File providers to run and make available prior to execution
    #[serde(default)]
    pub file_providers: Vec<NamedFileProvider>,
}

impl DockerScenario {
    fn as_command_and_args(
        &self,
        env_vars: &HashMap<String, String>,
        ctx: &impl ResolutionContext,
    ) -> (&'static str, Vec<String>) {
        let image = match self.tag.as_ref() {
            Some(tag) => format!("{}:{}", self.image.as_resolved(), tag.as_resolved()),
            None => self.image.as_resolved().to_string(),
        };

        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-v".to_string(),
            format!("{}:/output", ctx.output_path().display()),
        ];

        for (k, v) in env_vars.iter() {
            args.extend(["-e".to_string(), format!("{k}={v}")]);
        }

        args.extend([
            image,
            "sh".to_string(),
            "-c".to_string(),
            self.command.as_resolved().to_string(),
        ]);

        ("docker", args)
    }

    /// Combine the base environment variables we have with the ones coming from the file providers
    /// we need to run. The `out_dir` argument here needs to match the one used when running
    /// and outputting the content of the file providers.
    pub fn all_env_vars(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars = self.file_path_env_vars(out_dir, output_path, ctx)?;

        let out_dir = ctx.output_path().to_string_lossy();
        for path in vars.values_mut() {
            let in_container_path = match path.strip_prefix(out_dir.as_ref()) {
                Some(tail) => format!("/output{tail}"),
                None => {
                    panic!("provider path found that is outside of the output directory: {path}")
                }
            };

            *path = in_container_path;
        }

        vars.extend(self.explicit_env_vars());

        Ok(vars)
    }

    fn explicit_env_vars(&self) -> HashMap<String, String> {
        self.env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_resolved().to_string()))
            .collect()
    }

    fn file_path_env_vars(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars = HashMap::with_capacity(self.file_providers.len() + 2);

        for nfp in self.file_providers.iter() {
            let path = ctx
                .known_provider_output_path(Provider::File { fp: &nfp.provider })
                .ok_or(providers::Error::MissingProviderOutput {
                    name: nfp.name.clone(),
                })?;
            vars.insert(nfp.env_var.clone(), path.to_string_lossy().to_string());
        }

        vars.insert(OUTDIR.to_string(), out_dir.display().to_string());
        vars.insert(OUTPUT_PATH.to_string(), output_path.display().to_string());

        Ok(vars)
    }
}

impl Execute for DockerScenario {
    fn command_name(&self) -> &str {
        "docker"
    }

    fn execute(
        &self,
        out_dir: &Path,
        output_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        let env_vars = self.all_env_vars(out_dir, output_path, ctx)?;
        let (cmd, args) = self.as_command_and_args(&env_vars, ctx);

        ctx.run_command_blocking(cmd, args.iter().map(|s| s.as_str()), &HashMap::default())
            .map_err(Into::into)
    }
}

impl RunProviders for DockerScenario {
    async fn run_providers(
        &self,
        providers_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        self.file_providers.run_providers(providers_dir, ctx).await
    }

    fn inline<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        self.file_providers.inline(ctx)
    }

    fn inline_all_relative_paths<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + 'a>> {
        self.file_providers.inline_all_relative_paths(ctx)
    }
}

impl Check for DockerScenario {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        // We deliberately _don't_ check for the presence of docker on the PATH as part of static
        // checks as we can't guarantee that checks are being run on the same system that will
        // ultimately execute the test plan.
        // This means that docker _not_ being on the PATH will result in a runtime error during
        // command execution, which matches the behaviour of missing dependencies in other
        // CommandProivider variants where we don't even know what dependencies the script has.

        Ok(())
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
            command: ScenarioCommand::Script(CommandSection {
                file_providers: named_file_providers_with_fields(fields),
                ..CommandSection::empty()
            }),
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
            command: ScenarioCommand::Script(CommandSection {
                file_providers: templatable_file_providers(scenario_fields),
                ..CommandSection::empty()
            }),
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
            command::{
                CommandProvider, CommandSection, CommandSpec,
                test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            },
            file::{InlineFile, RawSource, RelativeFile, SourceDir},
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
            allowed_values: ["foo1", "foo2"]
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

        let res = scenario.try_template(&mut Vec::new(), &SourceDir::local("/"), &ctx);
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
            expected_error_details(expected_err_fields, "scenario");

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
            command: ScenarioCommand::Script(cmd_with_inline_file()),
            ..ScenarioConfig::empty()
        };

        let ctx = Context::new();

        let res = scenario.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn check_command_errors() {
        let scenario = ScenarioConfig {
            command: ScenarioCommand::Script(cmd_with_required_file()),
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
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
            .await
            .expect("custom providers should load successfully");

        assert_eq!(loaded_providers.len(), 2);

        let (provider1_source, provider1_def) = loaded_providers
            .get("provider1")
            .expect("provider1 should exist");
        assert_eq!(
            provider1_source,
            &SourceDir::local(providers_dir.canonicalize().unwrap())
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
            &SourceDir::local(providers_dir.canonicalize().unwrap())
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
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
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
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
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
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
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
            .try_load_all(&SourceDir::local(temp.path()), &Context::new())
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

    #[tokio::test]
    async fn scenario_config_inline_succeeds() {
        let file_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", file_content);

        let ctx = Context::new();
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let mut scenario = ScenarioConfig {
            command: ScenarioCommand::Script(CommandSection {
                command: CommandSpec {
                    name: "example.sh".to_string(),
                    command_provider: CommandProvider::RelativePath(RelativeFile {
                        path: Field::Resolved("file.txt".to_string()),
                        src: Some(src.clone()),
                    }),
                    args: Vec::new(),
                },
                ..CommandSection::empty()
            }),
            ..ScenarioConfig::empty()
        };

        let result = scenario.inline(&ctx).await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");
        let command_provider = match scenario.command {
            ScenarioCommand::Script(inner) => inner.command.command_provider,
            ScenarioCommand::Docker(_) => panic!("scenario command should be a script"),
        };

        assert_eq!(
            command_provider,
            CommandProvider::Inline(InlineFile {
                content: file_content.to_string(),
            }),
            "Expected command provider to be inlined"
        );
    }
}
