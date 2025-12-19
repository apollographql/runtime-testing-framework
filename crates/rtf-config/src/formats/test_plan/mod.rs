use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    formats::{
        CustomProviderDeclaration, EnvironmentConfig, Error, Matrix, Result, ScenarioConfig,
    },
    providers::file::SourceDir,
    templating::{self, Scalar, Template, TemplateContext},
};
use rtf_integrations::github::{self, Client};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

mod validation;

pub(crate) mod raw;
pub(crate) mod sources;

pub use raw::RawTestPlanConfig;
pub(crate) use raw::strip_sources_for_relative_paths;
pub use sources::Sources;

// Namespace directories for containing the file provider output from each command section
const SETUP_PROVIDER_DIR: &str = "setup";
const SCENARIO_PROVIDER_DIR: &str = "scenario";
const TEARDOWN_PROVIDER_DIR: &str = "teardown";

/// The format for parsing scenario config
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TestPlanConfig {
    pub name: String,
    pub description: String,
    #[serde(default, alias = "values")]
    // This alias is for backwards compatibility with the original name
    pub variables: HashMap<String, Scalar>,
    #[serde(default)]
    pub matrix: Matrix,
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderDeclaration>,
    pub scenario: ScenarioConfig,
    pub environment: EnvironmentConfig,
    #[serde(skip)]
    pub sources: Sources,
}

impl TestPlanConfig {
    pub async fn try_load_and_resolve_from_path(
        p: impl AsRef<Path>,
        ctx: &impl ResolutionContext,
    ) -> Result<Self> {
        let content = ctx.read_path_to_string(p.as_ref())?;
        let raw: RawTestPlanConfig = serde_yaml::from_str(&content)?;
        let abs_path = ctx.canonicalize_path(p.as_ref())?;
        let tp_source = SourceDir::local(abs_path.parent().unwrap());

        raw.try_into_test_plan(tp_source, ctx).await
    }

    pub async fn try_load_and_resolve_from_github(
        org: &str,
        repo: &str,
        path: &str,
        git_ref: Option<String>,
        ctx: &impl ResolutionContext,
    ) -> Result<Self> {
        let client = match ctx.github_client() {
            Some(client) => client,
            None => return Err(github::Error::NoClient.into()),
        };

        let content = client
            .string_file_content(org, repo, path, git_ref.as_ref())
            .await?;

        let raw: RawTestPlanConfig = serde_yaml::from_str(&content)?;
        let tp_source =
            SourceDir::github(org, repo, PathBuf::from(path).parent().unwrap(), git_ref);

        raw.try_into_test_plan(tp_source, ctx).await
    }

    /// Iteratate over all variants of this test plan that arise from [expanding](Matrix::try_expand)
    /// any matrix variables that it contains.
    ///
    /// This will always return at least the base test plan itself if there are no matrix variables
    /// defined.
    pub fn try_iter_matrix_variants(&self) -> Result<impl Iterator<Item = (String, Self)>> {
        let expanded = self.matrix.try_expand(&self.variables)?;

        Ok(expanded.into_iter().map(|(name, variables)| {
            let mut new = self.clone();
            new.variables = variables;
            new.matrix.clear();

            (name, new)
        }))
    }

    pub fn try_template_environment_setup(
        &mut self,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut path = vec!["environment".to_string()];
        self.environment
            .try_template_setup(&mut path, self.sources.environment(), ctx)
    }

    pub fn try_template_environment_teardown(
        &mut self,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut path = vec!["environment".to_string()];
        self.environment
            .try_template_teardown(&mut path, self.sources.environment(), ctx)
    }

    pub fn try_template_scenario(&mut self, ctx: &TemplateContext) -> templating::Result<()> {
        let mut path = vec!["scenario".to_string()];
        self.scenario
            .try_template(&mut path, self.sources.scenario(), ctx)
    }

    pub async fn run_environment_setup(
        &self,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> Result<HashMap<String, Scalar>> {
        let raw_output = self
            .environment
            .setup
            .command
            .run_providers_and_execute_for_output(SETUP_PROVIDER_DIR, out_dir, ctx)
            .await?;

        let provides: HashMap<String, Scalar> = match serde_json::from_str(&raw_output) {
            Ok(p) => p,
            Err(_e) => return Err(Error::MalformedSetupOutputFormat { output: raw_output }),
        };

        let mut missing = Vec::new();
        for val in self.environment.setup.provides.iter() {
            if !provides.contains_key(&val.name) {
                missing.push(val.name.clone());
            }
        }

        if missing.is_empty() {
            Ok(provides)
        } else {
            Err(Error::MissingSetupOutputFields { missing })
        }
    }

    pub async fn run_environment_teardown(
        &self,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> Result<()> {
        self.environment
            .teardown
            .run_providers_and_execute_for_output(TEARDOWN_PROVIDER_DIR, out_dir, ctx)
            .await?;

        Ok(())
    }

    pub async fn run_scenario(
        &self,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> Result<()> {
        self.scenario
            .command
            .run_providers_and_execute_for_output(SCENARIO_PROVIDER_DIR, out_dir, ctx)
            .await?;

        Ok(())
    }

    /// Create an empty [TestPlanConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> TestPlanConfig {
        TestPlanConfig {
            name: Default::default(),
            description: Default::default(),
            variables: Default::default(),
            matrix: Default::default(),
            custom_providers: Default::default(),
            scenario: ScenarioConfig::empty(),
            environment: EnvironmentConfig::empty(),
            sources: Sources::default(),
        }
    }

    pub fn as_yaml_string_without_sources(&self) -> Result<String> {
        let mut val = serde_yaml::to_value(self)?;
        raw::strip_sources_for_relative_paths(&mut val);

        Ok(serde_yaml::to_string(&val)?)
    }
}

impl Template for TestPlanConfig {
    fn has_pending_fields(&self) -> bool {
        self.environment.has_pending_fields() || self.scenario.has_pending_fields()
    }

    fn required_variables(&self) -> Vec<String> {
        let mut vals = self.environment.required_variables();
        vals.extend(self.scenario.required_variables());

        vals
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        _allowed_variables: &HashSet<&String>,
        _file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let allowed_variables = self.allowed_variables();
        let mut errs = templating::ErrorBuilder::from(self.environment.validate_context_nested(
            path,
            "environment",
            &allowed_variables,
            self.sources.environment(),
            ctx,
        ));
        errs.append(self.scenario.validate_context_nested(
            path,
            "scenario",
            &allowed_variables,
            self.sources.scenario(),
            ctx,
        ));

        errs.into_result(())
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        _source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.environment.try_template_nested(
            path,
            "environment",
            self.sources.environment(),
            ctx,
        ));
        errs.append(self.scenario.try_template_nested(
            path,
            "scenario",
            self.sources.scenario(),
            ctx,
        ));

        errs.into_result(())
    }
}

impl Check for TestPlanConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs =
            checks::ErrorBuilder::from(self.environment.try_check_nested(path, "environment", ctx));
        errs.append(self.scenario.try_check_nested(path, "scenario", ctx));

        errs.into_result(())
    }
}

#[cfg(test)]
pub(super) mod test_helpers {
    /// Create a HashMap of variables from key-value pairs using try_from
    #[macro_export]
    macro_rules! variables_map {
        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();
            $( m.insert($k.to_string(), $crate::templating::Scalar::try_from($v).unwrap()); )+
            m
        }};
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        formats::{
            environment::{SetupSection, test_helpers::environment_with_fields},
            scenario::test_helpers::scenario_with_fields,
            tests::{
                assert_check_errors, assert_template_errors, expected_error_details, p, r,
                templatable_file_providers, template_context, variable_definitions,
            },
        },
        providers::{
            command::{
                CommandProvider, CommandSection, CommandSpec,
                test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            },
            file::{FileProvider, InlineFile, NamedFileProvider, RawSource},
        },
        templating::Field,
        variables_map,
    };
    use assert_fs::{
        TempDir,
        prelude::{FileWriteStr, PathChild, PathCreateDir},
    };
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::path::PathBuf;

    // Helper functions

    /// Create a TestPlanConfig for testing Template trait methods (has_pending_fields, required_variables)
    fn test_plan_with_fields(
        scenario_fields: &[Field<String>],
        environment_fields: &[Field<String>],
        custom_providers: &[CustomProviderDeclaration],
    ) -> TestPlanConfig {
        TestPlanConfig {
            custom_providers: custom_providers.to_vec(),
            scenario: scenario_with_fields(scenario_fields, &[]),
            environment: environment_with_fields(&[], environment_fields, &[]),
            ..TestPlanConfig::empty()
        }
    }

    // Tests for configuration parsing from inline YAML and external files

    const RAW_TEST_PLAN_WITH_CUSTOM_PROVIDERS: &str = indoc!(
        r#"
            name: test-plan-with-custom-providers
            description: test plan with custom providers
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
            scenario:
              inline:
                name: scenario
                description: a scenario
                command:
                  name: scenario.sh
                  kind: inline
                  content: |
                    #!/usr/bin/env sh
                    echo "Hello!"
            environment:
              inline:
                name: environment
                description: an environment
                setup:
                  command:
                    name: setup.sh
                    kind: inline
                    content: |
                      #!/usr/bin/env sh
                      echo "Setup!"
                teardown:
                  command:
                    name: teardown.sh
                    kind: inline
                    content: |
                      #!/usr/bin/env sh
                      echo "Teardown!"
        "#
    );

    const INLINE_TEST_PLAN: &str = indoc!(
        r#"
            name: inline-test-plan
            description: test plan with inline scenario and environment
            variables:
              foo: "foo"
              bar: "bar"
            scenario:
              inline: 
                name: inline-scenario
                description: an inline scenario
                variable_definitions:
                  - name: foo
                    description: a value foo
                command: 
                  name: scenario.sh
                  kind: inline
                  content: |
                    #!/usr/bin/env sh
                    echo "Hello, World!"
                env_vars:
                  FOO: "{{ foo }}"
            environment:
              inline:
                name: inline-environment
                description: an inline environment
                variable_definitions:
                  - name: bar
                    description: a value bar
                setup:
                  command:
                    name: setup.sh
                    kind: inline
                    content: |
                      #!/usr/bin/env sh
                      echo "Hello, world!"
                  env_vars:
                    BAR: "{{ bar }}"
                  provides:
                    - name: baz
                      description: a value baz
                teardown:
                  command:
                    name: teardown.sh
                    kind: inline
                    content: |
                      #!/usr/bin/env sh
                      echo "Hello, world!"
                  env_vars:
                    BAZ: "{{ baz }}"
        "#
    );

    #[tokio::test]
    async fn parse_and_template_inline_config() {
        let raw_test_plan: RawTestPlanConfig =
            serde_yaml::from_str(INLINE_TEST_PLAN).expect("test plan config to parse");
        let ctx = Context::new();

        let expected_sources = Sources::with_custom_providers(
            SourceDir::Local {
                abs_path: "/".into(),
            },
            None,
            None,
            Default::default(),
        );

        let res = raw_test_plan
            .try_into_test_plan(
                SourceDir::Local {
                    abs_path: "/".into(),
                },
                &ctx,
            )
            .await;
        assert!(res.is_ok(), "expected TestPlanConfig, got {res:?}");

        let test_plan = res.unwrap();
        let sources = test_plan.clone().sources;
        assert_eq!(
            sources, expected_sources,
            "test that sources are set correctly"
        );

        let res = test_plan.required_variables();
        assert_eq!(
            res,
            &["bar", "baz", "foo"],
            "check that test plan returns fields"
        )
    }

    #[test]
    fn parse_custom_providers() {
        let config: RawTestPlanConfig = serde_yaml::from_str(RAW_TEST_PLAN_WITH_CUSTOM_PROVIDERS)
            .expect("test plan config to parse");

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

    #[tokio::test]
    async fn custom_providers_all_levels_integration() {
        let temp = TempDir::new().unwrap();

        let test_plan_providers = temp.child("test_plan_providers");
        test_plan_providers.create_dir_all().unwrap();

        let scenario_providers = temp.child("scenario_providers");
        scenario_providers.create_dir_all().unwrap();

        let environment_providers = temp.child("environment_providers");
        environment_providers.create_dir_all().unwrap();

        let test_plan_provider_yaml = indoc!(
            r#"
                name: test-plan-provider
                description: provider from test plan level
                variable_definitions: []
                command:
                  name: test-plan.sh
                  kind: inline
                  content: |
                    #!/usr/bin/env sh
                    echo "test plan provider"
            "#
        );
        test_plan_providers
            .child("tp_provider.yaml")
            .write_str(test_plan_provider_yaml)
            .unwrap();

        let scenario_provider_yaml = indoc!(
            r#"
                name: scenario-provider
                description: provider from scenario level
                variable_definitions: []
                command:
                  name: scenario.sh
                  kind: inline
                  content: |
                    #!/usr/bin/env sh
                    echo "scenario provider"
            "#
        );
        scenario_providers
            .child("sc_provider.yaml")
            .write_str(scenario_provider_yaml)
            .unwrap();

        let environment_provider_yaml = indoc!(
            r#"
                name: environment-provider
                description: provider from environment level
                variable_definitions: []
                command:
                  name: env.sh
                  kind: inline
                  content: |
                    #!/usr/bin/env sh
                    echo "environment provider"
            "#
        );
        environment_providers
            .child("env_provider.yaml")
            .write_str(environment_provider_yaml)
            .unwrap();

        let test_plan_config = indoc!(
            r#"
                name: test-plan-with-all-levels
                description: test plan with custom providers at all levels
                custom_providers:
                  - kind: local
                    relative_path: test_plan_providers
                    using:
                      tp_provider: tp_provider.yaml
                scenario:
                  inline:
                    name: scenario
                    description: a scenario
                    custom_providers:
                      - kind: local
                        relative_path: scenario_providers
                        using:
                          sc_provider: sc_provider.yaml
                    command:
                      name: scenario.sh
                      kind: inline
                      content: |
                        #!/usr/bin/env sh
                        echo "Hello!"
                environment:
                  inline:
                    name: environment
                    description: an environment
                    custom_providers:
                      - kind: local
                        relative_path: environment_providers
                        using:
                          env_provider: env_provider.yaml
                    setup:
                      command:
                        name: setup.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Setup!"
                    teardown:
                      command:
                        name: teardown.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Teardown!"
            "#
        );

        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file.write_str(test_plan_config).unwrap();

        let ctx = Context::new();
        let res =
            TestPlanConfig::try_load_and_resolve_from_path(test_plan_file.to_path_buf(), &ctx)
                .await;

        assert!(res.is_ok(), "expected TestPlanConfig, got {res:?}");

        let test_plan = res.unwrap();
        let sources = &test_plan.sources;

        assert_eq!(sources.custom_providers().test_plan.len(), 1);
        assert!(
            sources
                .custom_providers()
                .test_plan
                .contains_key("tp_provider")
        );

        assert_eq!(sources.custom_providers().scenario.len(), 1);
        assert!(
            sources
                .custom_providers()
                .scenario
                .contains_key("sc_provider")
        );

        assert_eq!(sources.custom_providers().environment.len(), 1);
        assert!(
            sources
                .custom_providers()
                .environment
                .contains_key("env_provider")
        );
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
    async fn custom_providers_test_plan_level_missing_file_errors() {
        let temp = TempDir::new().unwrap();

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        let test_plan_config = indoc!(
            r#"
                name: test-plan-with-missing-provider
                description: test plan with missing provider file
                custom_providers:
                  - kind: local
                    relative_path: providers
                    using:
                      missing_provider: missing.yaml
                scenario:
                  inline:
                    name: scenario
                    description: a scenario
                    command:
                      name: scenario.sh
                      kind: inline
                      content: |
                        #!/usr/bin/env sh
                        echo "Hello!"
                environment:
                  inline:
                    name: environment
                    description: an environment
                    setup:
                      command:
                        name: setup.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Setup!"
                    teardown:
                      command:
                        name: teardown.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Teardown!"
            "#
        );

        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file.write_str(test_plan_config).unwrap();

        let ctx = Context::new();
        let res =
            TestPlanConfig::try_load_and_resolve_from_path(test_plan_file.to_path_buf(), &ctx)
                .await;

        assert!(res.is_err(), "expected error for missing file");

        match res {
            Err(Error::FailedCustomProviderDefinitions { errs }) => {
                assert_eq!(errs.len(), 1);
                assert!(
                    errs[0].contains("test plan:"),
                    "error should be prefixed with 'test plan:'"
                );
                assert!(
                    errs[0].contains("missing_provider"),
                    "error should mention the provider name"
                );
            }
            _ => panic!("expected FailedCustomProviderDefinitions error, got {res:?}"),
        }
    }

    #[tokio::test]
    async fn custom_providers_test_plan_level_invalid_yaml_errors() {
        let temp = TempDir::new().unwrap();

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        providers_dir
            .child("invalid.yaml")
            .write_str("not valid yaml: {{{]}")
            .unwrap();

        let test_plan_config = indoc!(
            r#"
                name: test-plan-with-invalid-provider
                description: test plan with invalid provider yaml
                custom_providers:
                  - kind: local
                    relative_path: providers
                    using:
                      invalid_provider: invalid.yaml
                scenario:
                  inline:
                    name: scenario
                    description: a scenario
                    command:
                      name: scenario.sh
                      kind: inline
                      content: |
                        #!/usr/bin/env sh
                        echo "Hello!"
                environment:
                  inline:
                    name: environment
                    description: an environment
                    setup:
                      command:
                        name: setup.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Setup!"
                    teardown:
                      command:
                        name: teardown.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Teardown!"
            "#
        );

        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file.write_str(test_plan_config).unwrap();

        let ctx = Context::new();
        let res =
            TestPlanConfig::try_load_and_resolve_from_path(test_plan_file.to_path_buf(), &ctx)
                .await;

        assert!(res.is_err(), "expected error for invalid YAML");

        match res {
            Err(Error::FailedCustomProviderDefinitions { errs }) => {
                assert_eq!(errs.len(), 1);
                assert!(
                    errs[0].contains("test plan:"),
                    "error should be prefixed with 'test plan:'"
                );
                assert!(
                    errs[0].contains("invalid_provider"),
                    "error should mention the provider name"
                );
            }
            _ => panic!("expected FailedCustomProviderDefinitions error, got {res:?}"),
        }
    }

    #[tokio::test]
    async fn custom_providers_test_plan_level_nested_custom_providers_errors() {
        let temp = TempDir::new().unwrap();

        let providers_dir = temp.child("providers");
        providers_dir.create_dir_all().unwrap();

        providers_dir
            .child("nested.yaml")
            .write_str(CUSTOM_PROVIDER_WITH_NESTED)
            .unwrap();

        let test_plan_config = indoc!(
            r#"
                name: test-plan-with-nested-provider
                description: test plan with nested custom provider
                custom_providers:
                  - kind: local
                    relative_path: providers
                    using:
                      nested_provider: nested.yaml
                scenario:
                  inline:
                    name: scenario
                    description: a scenario
                    command:
                      name: scenario.sh
                      kind: inline
                      content: |
                        #!/usr/bin/env sh
                        echo "Hello!"
                environment:
                  inline:
                    name: environment
                    description: an environment
                    setup:
                      command:
                        name: setup.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Setup!"
                    teardown:
                      command:
                        name: teardown.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Teardown!"
            "#
        );

        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file.write_str(test_plan_config).unwrap();

        let ctx = Context::new();
        let res =
            TestPlanConfig::try_load_and_resolve_from_path(test_plan_file.to_path_buf(), &ctx)
                .await;

        assert!(res.is_err(), "expected error for nested custom providers");

        match res {
            Err(Error::FailedCustomProviderDefinitions { errs }) => {
                assert_eq!(errs.len(), 1);
                assert!(
                    errs[0].contains("test plan:"),
                    "error should be prefixed with 'test plan:'"
                );
                assert!(
                    errs[0].contains("nested_provider"),
                    "error should mention the provider name"
                );
            }
            _ => panic!("expected FailedCustomProviderDefinitions error, got {res:?}"),
        }
    }

    #[tokio::test]
    async fn custom_providers_mixed_levels_with_errors() {
        let temp = TempDir::new().unwrap();

        let test_plan_providers = temp.child("test_plan_providers");
        test_plan_providers.create_dir_all().unwrap();

        let scenario_providers = temp.child("scenario_providers");
        scenario_providers.create_dir_all().unwrap();

        let environment_providers = temp.child("environment_providers");
        environment_providers.create_dir_all().unwrap();

        scenario_providers
            .child("invalid.yaml")
            .write_str("not valid yaml: {{{]}")
            .unwrap();

        environment_providers
            .child("nested.yaml")
            .write_str(CUSTOM_PROVIDER_WITH_NESTED)
            .unwrap();

        let test_plan_config = indoc!(
            r#"
                name: test-plan-with-mixed-errors
                description: test plan with errors at all levels
                custom_providers:
                  - kind: local
                    relative_path: test_plan_providers
                    using:
                      missing_provider: missing.yaml
                scenario:
                  inline:
                    name: scenario
                    description: a scenario
                    custom_providers:
                      - kind: local
                        relative_path: scenario_providers
                        using:
                          invalid_provider: invalid.yaml
                    command:
                      name: scenario.sh
                      kind: inline
                      content: |
                        #!/usr/bin/env sh
                        echo "Hello!"
                environment:
                  inline:
                    name: environment
                    description: an environment
                    custom_providers:
                      - kind: local
                        relative_path: environment_providers
                        using:
                          nested_provider: nested.yaml
                    setup:
                      command:
                        name: setup.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Setup!"
                    teardown:
                      command:
                        name: teardown.sh
                        kind: inline
                        content: |
                          #!/usr/bin/env sh
                          echo "Teardown!"
            "#
        );

        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file.write_str(test_plan_config).unwrap();

        let ctx = Context::new();
        let res =
            TestPlanConfig::try_load_and_resolve_from_path(test_plan_file.to_path_buf(), &ctx)
                .await;

        match res {
            Err(Error::FailedCustomProviderDefinitions { errs }) => {
                assert_eq!(errs.len(), 3, "expected errors from all three levels");

                let has_test_plan_error = errs.iter().any(|e| e.contains("test plan:"));
                let has_scenario_error = errs.iter().any(|e| e.contains("scenario:"));
                let has_environment_error = errs.iter().any(|e| e.contains("environment:"));

                assert!(has_test_plan_error);
                assert!(has_scenario_error);
                assert!(has_environment_error);

                let error_str = errs.join("\n");
                assert!(error_str.contains("missing_provider"));
                assert!(error_str.contains("invalid_provider"));
                assert!(error_str.contains("nested_provider"));
            }

            _ => panic!("expected FailedCustomProviderDefinitions error, got {res:?}"),
        }
    }

    const TEST_PLAN_WITH_SCENARIO_CUSTOM_PROVIDER_OVERRIDE: &str = indoc!(
        r#"
            name: test-plan-scenario-override-custom-providers
            description: test plan with scenario overrides containing custom_providers
            scenario:
              from:
                kind: local
                relative_path: scenario.yaml
              overrides:
                custom_providers:
                  - kind: local
                    relative_path: providers
                    using:
                      my_provider: provider.yaml
            environment:
              inline:
                name: environment
                description: an environment
                setup:
                  command:
                    name: setup.sh
                    kind: inline
                    content: |
                      #!/usr/bin/env sh
                      echo "Setup!"
                teardown:
                  command:
                    name: teardown.sh
                    kind: inline
                    content: |
                      #!/usr/bin/env sh
                      echo "Teardown!"
        "#
    );

    const TEST_PLAN_WITH_ENVIRONMENT_CUSTOM_PROVIDER_OVERRIDE: &str = indoc!(
        r#"
            name: test-plan-environment-override-custom-providers
            description: test plan with environment overrides containing custom_providers
            scenario:
              inline:
                name: scenario
                description: a scenario
                command:
                  name: scenario.sh
                  kind: inline
                  content: |
                    #!/usr/bin/env sh
                    echo "Hello!"
            environment:
              from:
                kind: local
                relative_path: environment.yaml
              overrides:
                custom_providers:
                  - kind: local
                    relative_path: providers
                    using:
                      my_provider: provider.yaml
        "#
    );

    #[test_case(TEST_PLAN_WITH_SCENARIO_CUSTOM_PROVIDER_OVERRIDE; "scenario overrides with custom providers")]
    #[test_case(TEST_PLAN_WITH_ENVIRONMENT_CUSTOM_PROVIDER_OVERRIDE; "environment overrides with custom providers")]
    #[tokio::test]
    async fn custom_providers_in_overrides_should_error(test_plan_yaml: &str) {
        let temp = TempDir::new().unwrap();

        let scenario_file = temp.child("scenario.yaml");
        scenario_file
            .write_str(&serde_yaml::to_string(&ScenarioConfig::empty()).unwrap())
            .unwrap();

        let environment_file = temp.child("environment.yaml");
        environment_file
            .write_str(&serde_yaml::to_string(&EnvironmentConfig::empty()).unwrap())
            .unwrap();

        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file.write_str(test_plan_yaml).unwrap();

        let ctx = Context::new();
        let res =
            TestPlanConfig::try_load_and_resolve_from_path(test_plan_file.to_path_buf(), &ctx)
                .await;

        match res {
            Err(Error::InvalidCustomProviderOverride) => (),
            _ => panic!("expected InvalidCustomProviderOverride error, got {res:?}"),
        }
    }

    const FROM_SAME_DIR_FILES_TEST_PLAN: &str = indoc!(
        r#"
            name: from-files-test-plan
            description: test plan with scenario and environment from files
            variables:
              foo: "foo"
              bar: "bar"
            scenario:
              from:
                kind: local
                relative_path: scenario.yaml
            environment:
              from:
                kind: local
                relative_path: environment.yaml
        "#
    );

    const FROM_NESTED_FILE_PATHS_TEST_PLAN: &str = indoc!(
        r#"
            name: from-files-test-plan
            description: test plan with scenario and environment from files
            variables:
              foo: "foo"
              bar: "bar"
            scenario:
              from:
                kind: local
                relative_path: ../../scenario.yaml
            environment:
              from:
                kind: local
                relative_path: ./nested/environment.yaml
        "#
    );

    #[test_case(FROM_SAME_DIR_FILES_TEST_PLAN, "", "", ""; "environment and scenario in same directory")]
    #[test_case(FROM_NESTED_FILE_PATHS_TEST_PLAN, "foo/bar/", "", "foo/bar/nested/"; "environment and scenario in nested directories")]
    #[tokio::test]
    async fn parse_test_plan_config_from_files(
        test_plan: &str,
        test_plan_path: &str,
        scenario_path: &str,
        environment_path: &str,
    ) {
        let temp = TempDir::new().unwrap();

        let tp_file = temp.child(format!("{}test-plan.yaml", test_plan_path));
        let scenario_file = temp.child(format!("{}scenario.yaml", scenario_path));
        let environment_file = temp.child(format!("{}environment.yaml", environment_path));

        tp_file
            .write_str(test_plan)
            .expect("failed to write test plan file");
        scenario_file
            .write_str(&serde_yaml::to_string(&ScenarioConfig::empty()).unwrap())
            .expect("failed to write scenario file");
        environment_file
            .write_str(&serde_yaml::to_string(&EnvironmentConfig::empty()).unwrap())
            .expect("failed to write environment file");

        let ctx = Context::new();

        let config_file_dir = |p: &Path| {
            ctx.canonicalize_path(p)
                .unwrap()
                .parent()
                .unwrap()
                .to_owned()
        };

        let expected_sources = Sources::with_custom_providers(
            SourceDir::Local {
                abs_path: config_file_dir(&tp_file),
            },
            Some(SourceDir::Local {
                abs_path: config_file_dir(&scenario_file),
            }),
            Some(SourceDir::Local {
                abs_path: config_file_dir(&environment_file),
            }),
            Default::default(),
        );

        let res = TestPlanConfig::try_load_and_resolve_from_path(tp_file.to_path_buf(), &ctx).await;
        assert!(res.is_ok(), "expected TestPlanConfig, got {res:?}");

        let test_plan = res.unwrap();
        let sources = test_plan.clone().sources;
        assert_eq!(
            sources, expected_sources,
            "test that sources are set correctly"
        );

        let res = test_plan.required_variables();
        let expected_variables: &[&str] = &[];
        assert_eq!(
            res, expected_variables,
            "check that test plan returns fields"
        )
    }

    const OVERRIDES_TEST_PLAN: &str = indoc!(
        r#"
            name: from-files-test-plan
            description: test plan with scenario and environment from files
            variables:
              foo: "foo"
              bar: "bar"
            scenario:
              from:
                kind: local
                relative_path: scenario.yaml
              overrides:
                name: scenario
                command: 
                  name: scenario.sh
                  kind: inline
                  content: |
                    #!/usr/bin/env sh
                    echo "Hello, World!"
            environment:
              from:
                kind: local
                relative_path: environment.yaml
              overrides:
                setup:
                  file_providers:
                    - name: file.txt
                      env_var: FILE
                      kind: inline
                      content: |
                        some inline text
        "#
    );

    #[tokio::test]
    async fn parse_with_overrides() {
        let temp = TempDir::new().unwrap();

        let tp_file = temp.child("test-plan.yaml");
        let scenario_file = temp.child("scenario.yaml");
        let environment_file = temp.child("environment.yaml");

        tp_file
            .write_str(OVERRIDES_TEST_PLAN)
            .expect("failed to write test plan file");
        scenario_file
            .write_str(&serde_yaml::to_string(&ScenarioConfig::empty()).unwrap())
            .expect("failed to write scenario file");
        environment_file
            .write_str(&serde_yaml::to_string(&EnvironmentConfig::empty()).unwrap())
            .expect("failed to write environment file");

        let expected_scenario_name = "scenario";
        let expected_scenario_command = CommandSection {
            command: CommandSpec {
                name: "scenario.sh".to_string(),
                args: Vec::new(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: "#!/usr/bin/env sh\necho \"Hello, World!\"\n".to_string(),
                }),
            },
            ..CommandSection::empty()
        };
        let expected_env_files = vec![NamedFileProvider {
            name: "file.txt".to_string(),
            env_var: "FILE".to_string(),
            provider: FileProvider::Inline(InlineFile {
                content: "some inline text\n".to_string(),
            }),
        }];

        let ctx = Context::new();

        let res = TestPlanConfig::try_load_and_resolve_from_path(tp_file.to_path_buf(), &ctx).await;
        assert!(res.is_ok(), "expected TestPlanConfig, got {res:?}");

        let test_plan = res.unwrap();
        let scenario_name = &test_plan.scenario.name;
        assert_eq!(
            scenario_name, &expected_scenario_name,
            "test the scenario name comes from overrides"
        );

        let scenario_command = &test_plan.scenario.command;
        assert_eq!(
            scenario_command, &expected_scenario_command,
            "test the scenario command comes from overrides"
        );

        let environment_files = &test_plan.environment.setup.command.file_providers;
        assert_eq!(
            environment_files, &expected_env_files,
            "test the environment setup files come from overrides"
        );

        let res = test_plan.required_variables();
        let expected_variables: &[&str] = &[];
        assert_eq!(
            res, expected_variables,
            "check that test plan returns fields"
        )
    }

    // Tests for the Template trait implementations and field resolution
    #[test_case(p("foo"), p("bar"), true; "scenario and environment have pending fields is pending")]
    #[test_case(p("foo"), r("bar"), true; "scenario has pending field is pending")]
    #[test_case(r("foo"), p("bar"), true; "environment has pending field is pending")]
    #[test_case(r("foo"), r("bar"), false; "scenario and environment have no pending fields is resolved")]
    #[test]
    fn has_pending_fields(
        scenario_field: Field<String>,
        environment_field: Field<String>,
        expected: bool,
    ) {
        let test_plan = test_plan_with_fields(&[scenario_field], &[environment_field], &[]);

        let res = test_plan.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_variables has expected value"
        )
    }

    #[test_case(p("scenario"), p("environment"), &["environment", "scenario"]; "scenario and environment fields required")]
    #[test_case(p("scenario"), r("environment"), &["scenario"]; "scenario field required")]
    #[test_case(r("scenario"), p("environment"), &["environment"]; "environment field required")]
    #[test_case(r("scenario"), r("environment"), &[]; "no fields required")]
    #[test]
    fn required_variables(
        scenario_field: Field<String>,
        environment_field: Field<String>,
        expected: &[&str],
    ) {
        let test_plan = test_plan_with_fields(&[scenario_field], &[environment_field], &[]);

        let res = test_plan.required_variables();
        assert_eq!(
            res, expected,
            "tests that required_variables has expected value"
        )
    }

    #[test_case(&["scenario"], &["setup"], &["teardown"]; "scenario and setup and teardown have fields")]
    #[test_case(&["scenario"], &["setup"], &[]; "scenario and setup have fields")]
    #[test_case(&[], &["setup"], &["teardown"]; "setup and teardown have fields")]
    #[test_case(&["scenario"], &[], &["teardown"]; "scenario and teardown have fields")]
    #[test_case(&["scenario"], &[], &[]; "scenario has fields")]
    #[test_case(&[], &["setup"], &[]; "setup has fields")]
    #[test_case(&[], &[], &["teardown"]; "teardown has fields")]
    #[test_case(&[], &[], &[]; "no fields")]
    #[test_case(&["foo", "bar", "baz"], &[], &[]; "scenario multi variable and no environment")]
    #[test_case(&[], &["foo", "bar"], &[]; "setup multi variable and no teardown")]
    #[test_case(&[], &[], &["foo", "bar"]; "teardown multi variable and no setup")]
    #[test_case(&["s1", "s2"], &["setup1", "setup2"], &[]; "scenario and setup multi variable")]
    #[test_case(&["s1", "s2"], &[], &["teardown1", "teardown2"]; "scenario and teardown multi variable")]
    #[test_case(&[], &["setup1", "setup2"], &["teardown1", "teardown2"]; "setup and teardown multi variable")]
    #[test_case(&["s1", "s2"], &["setup1", "setup2"], &["teardown1", "teardown2"]; "all sections multi variable")]
    #[test]
    fn try_template_succeeds(
        scenario_fields: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
    ) {
        let mut env_fields = setup_fields.to_vec();
        env_fields.extend_from_slice(teardown_fields);
        let mut all_fields = scenario_fields.to_vec();
        all_fields.extend(&env_fields);

        let mut test_plan = TestPlanConfig {
            scenario: ScenarioConfig {
                variable_definitions: variable_definitions(scenario_fields),
                command: CommandSection {
                    file_providers: templatable_file_providers(scenario_fields),
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            environment: EnvironmentConfig {
                variable_definitions: variable_definitions(env_fields.as_slice()),
                teardown: CommandSection {
                    file_providers: templatable_file_providers(env_fields.as_slice()),
                    ..CommandSection::empty()
                },
                ..EnvironmentConfig::empty()
            },
            ..TestPlanConfig::empty()
        };
        let result = test_plan.try_template(
            &mut Vec::new(),
            &SourceDir::local("/"),
            &template_context(all_fields.as_slice()),
        );

        assert!(
            result.is_ok(),
            "expected templating to succeed, got {:?}",
            result
        );
    }

    /// Helper for creating a test plan for Template tests
    fn template_test_plan(
        scenario_variable_defs: &[&str],
        scenario_fields: &[&str],
        env_variable_defs: &[&str],
        env_fields: &[&str],
    ) -> TestPlanConfig {
        TestPlanConfig {
            scenario: ScenarioConfig {
                variable_definitions: variable_definitions(scenario_variable_defs),
                command: CommandSection {
                    file_providers: templatable_file_providers(scenario_fields),
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            environment: EnvironmentConfig {
                variable_definitions: variable_definitions(env_variable_defs),
                teardown: CommandSection {
                    file_providers: templatable_file_providers(env_fields),
                    ..CommandSection::empty()
                },
                ..EnvironmentConfig::empty()
            },
            ..TestPlanConfig::empty()
        }
    }

    /// Helper function for asserting template errors are as expected
    fn assert_test_plan_template_errors(
        test_plan: &mut TestPlanConfig,
        ctx: TemplateContext,
        expected_scenario_err_fields: &[&str],
        expected_env_err_fields: &[&str],
    ) {
        let (mut expected_err_messages, mut expected_err_paths) =
            expected_error_details(expected_scenario_err_fields, "scenario.command_section");
        let (expected_messages, expected_paths) =
            expected_error_details(expected_env_err_fields, "environment.teardown");
        expected_err_messages.extend(expected_messages);
        expected_err_paths.extend(expected_paths);
        expected_err_messages.sort();
        expected_err_paths.sort();

        assert_template_errors(test_plan, ctx, expected_err_messages, expected_err_paths);
    }

    #[test_case(&["missing"], &["scenario"], &["scenario"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["scenario1", "scenario2"], &["scenario1", "scenario2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["scenario1", "missing2"], &["scenario1", "scenario2"], &["scenario2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_scenario_missing_variable_definitions(
        variable_defs: &[&str],
        fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["scenario", "scenario1", "scenario2"]);
        let mut test_plan = template_test_plan(variable_defs, fields, &[], &[]);

        assert_test_plan_template_errors(&mut test_plan, ctx, expected_err_fields, &[]);
    }

    #[test_case(&["missing"], &["environment"], &["environment"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["environment1", "environment2"], &["environment1", "environment2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["environment1", "missing2"], &["environment1", "environment2"], &["environment2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_environment_missing_variable_definitions(
        variable_defs: &[&str],
        fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["environment", "environment1", "environment2"]);
        let mut test_plan = template_test_plan(&[], &[], variable_defs, fields);

        assert_test_plan_template_errors(&mut test_plan, ctx, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_scenario_and_environment_variable_definitions() {
        let ctx = template_context(&["scenario", "environment"]);
        let mut test_plan = template_test_plan(&[], &["scenario"], &[], &["environment"]);

        assert_test_plan_template_errors(&mut test_plan, ctx, &["scenario"], &["environment"]);
    }

    #[test]
    fn try_template_missing_scenario_and_environment_variables_not_provided() {
        let ctx = template_context(&[]);
        let mut test_plan = template_test_plan(
            &["scenario"],
            &["scenario"],
            &["environment"],
            &["environment"],
        );

        assert_test_plan_template_errors(&mut test_plan, ctx, &["scenario"], &["environment"]);
    }

    // Tests for matrix expansion, variants, and matrix-related functionality

    #[test_case(
        &[],
        &[],
        &[],
        &[
            HashMap::new()
        ];
        "empty everything"
    )]
    #[test_case(
        &["foo", "bar"],
        &[],
        &[],
        &[
            variables_map!("foo" => "foo", "bar" => "bar")
        ];
        "just variables"
    )]
    #[test_case(
        &[],
        &[("key", vec!["a", "b", "c"])],
        &[],
        &[
            variables_map!("key" => "a"),
            variables_map!("key" => "b"), 
            variables_map!("key" => "c")
        ];
        "just matrix dimensions"
    )]
    #[test_case(
        &[],
        &[],
        &[variables_map!("foo" => "foo", "bar" => "bar")],
        &[variables_map!("foo" => "foo", "bar" => "bar")];
        "just include"
    )]
    #[test_case(
        &["foo"],
        &[("key", vec!["a", "b", "c"])],
        &[],
        &[
            variables_map!("foo" => "foo", "key" => "a"),
            variables_map!("foo" => "foo", "key" => "b"), 
            variables_map!("foo" => "foo", "key" => "c")
        ];
        "single key matrix with multiple entries and one variable"
    )]
    #[test_case(
        &[],
        &[("key1", vec!["a", "b", "c"]), ("key2", vec!["1", "2"])],
        &[],
        &[
            variables_map!("key1" => "a", "key2" => "1"),
            variables_map!("key1" => "a", "key2" => "2"),
            variables_map!("key1" => "b", "key2" => "1"),
            variables_map!("key1" => "b", "key2" => "2"),
            variables_map!("key1" => "c", "key2" => "1"),
            variables_map!("key1" => "c", "key2" => "2")
        ];
        "multiple keys with multiple entries and no variables"
    )]
    #[test_case(
        &["foo"],
        &[],
        &[variables_map!("bar" => "bar")],
        &[variables_map!("foo" => "foo", "bar" => "bar")];
        "single include and one variable"
    )]
    #[test_case(
        &[],
        &[("key1", vec!["a", "b", "c"])],
        &[variables_map!("bar" => "bar")],
        &[
            variables_map!("bar" => "bar", "key1" => "a"),
            variables_map!("bar" => "bar", "key1" => "b"),
            variables_map!("bar" => "bar", "key1" => "c"),
        ];
        "single include and single key matrix with multiple entries"
    )]
    #[test_case(
        &["foo"],
        &[("key1", vec!["a", "b", "c"])],
        &[variables_map!("bar" => "bar")],
        &[
            variables_map!("foo" => "foo", "bar" => "bar", "key1" => "a"),
            variables_map!("foo" => "foo", "bar" => "bar", "key1" => "b"),
            variables_map!("foo" => "foo", "bar" => "bar", "key1" => "c"),
        ];
        "single include single key matrix with multiple entries and one variable"
    )]
    #[test]
    fn matrix_expansion(
        variables: &[&str],
        dimensions: &[(&str, Vec<&str>)],
        include: &[HashMap<String, Scalar>],
        expected_variables_maps: &[HashMap<String, Scalar>],
    ) {
        let variables = template_context(variables);
        let dimensions: HashMap<String, Vec<Scalar>> = dimensions
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| Scalar::from(*s)).collect()))
            .collect();
        let test_plan = TestPlanConfig {
            variables: variables.variables().clone(),
            matrix: Matrix {
                variant_names: None,
                dimensions,
                include: include.to_vec(),
            },
            ..TestPlanConfig::empty()
        };

        let variants: Vec<_> = test_plan.try_iter_matrix_variants().unwrap().collect();
        assert_eq!(
            variants.len(),
            expected_variables_maps.len(),
            "test the variants from iter_matrix_variants has the xepcted combination count"
        );
        assert!(
            variants.iter().all(|(_, v)| v.matrix.is_empty()),
            "expected all variants to have an empty matrix"
        );

        let n_variants = test_plan.matrix.n_variants();
        assert_eq!(
            n_variants,
            expected_variables_maps.len(),
            "test that the number of variants generated using iter_matrix_variants matches n_matrix_variants"
        );

        // Get the expanded matrix variables to make sure this outputs the same variables as iter_matrix_variants
        let expanded_matrix_variables = test_plan
            .matrix
            .try_expand(&test_plan.variables)
            .expect("expansion to succeed");

        // Check each variant has the expected combinations in the order expected
        for (i, (_, variant)) in variants.iter().enumerate() {
            let expected_variables = expected_variables_maps[i].clone();
            let expanded_variables = expanded_matrix_variables[i].1.clone();

            assert_eq!(
                variant.variables, expected_variables,
                "test the combination matches the expected one"
            );
            assert_eq!(
                variant.variables, expanded_variables,
                "test the combination from iter_matrix_variants matches the combination in expanded_matrix_variants"
            );
        }
    }

    #[test]
    fn check_success() {
        let test_plan = TestPlanConfig {
            scenario: ScenarioConfig {
                command: cmd_with_inline_file(),
                ..ScenarioConfig::empty()
            },
            environment: EnvironmentConfig {
                teardown: cmd_with_inline_file(),
                ..EnvironmentConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let ctx = Context::new();

        let res = test_plan.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test_case(
        cmd_with_required_file(),
        CommandSection::empty(),
        &[checks::ErrorKind::RequiredFileMissing];
        "scenario only"
    )]
    #[test_case(
        CommandSection::empty(),
        cmd_with_required_file(),
        &[checks::ErrorKind::RequiredFileMissing];
        "environment only"
    )]
    #[test_case(
        cmd_with_required_file(),
        cmd_with_required_file(),
        &[checks::ErrorKind::RequiredFileMissing, checks::ErrorKind::RequiredFileMissing];
        "scenario and environment"
    )]
    #[test]
    fn try_check_errors(
        scenario_cmd: CommandSection,
        environment_cmd: CommandSection,
        expected_err_kinds: &[checks::ErrorKind],
    ) {
        let test_plan = TestPlanConfig {
            scenario: ScenarioConfig {
                command: scenario_cmd,
                ..ScenarioConfig::empty()
            },
            environment: EnvironmentConfig {
                teardown: environment_cmd,
                ..EnvironmentConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let ctx = Context::new();

        assert_check_errors(test_plan, &ctx, expected_err_kinds);
    }

    /// Helper function for environment setup provides
    fn environment_setup_provides(script: &str) -> TestPlanConfig {
        TestPlanConfig {
            environment: EnvironmentConfig {
                setup: SetupSection {
                    command: CommandSection {
                        command: CommandSpec {
                            name: "setup.sh".to_string(),
                            command_provider: CommandProvider::Inline(InlineFile {
                                content: script.to_string(),
                            }),
                            args: Vec::new(),
                        },
                        ..CommandSection::empty()
                    },
                    provides: variable_definitions(&["foo", "bar"]),
                },
                ..EnvironmentConfig::empty()
            },
            ..TestPlanConfig::empty()
        }
    }

    #[tokio::test]
    async fn run_environment_setup_provides_expected_variables() {
        let temp = TempDir::new().unwrap();
        let mut ctx = Context::new();

        let expected_provides = template_context(&["foo", "bar"]).variables().clone();

        let script = indoc!(
            r#"
            #!/usr/bin/env sh
            echo "{ \"foo\": \"foo\", \"bar\": \"bar\" }" >> "$RTF_OUTPUT"
            "#
        );
        let test_plan = environment_setup_provides(script);

        let res = test_plan.run_environment_setup(&temp, &mut ctx).await;
        assert!(
            res.is_ok(),
            "expected a map of provides variables, got {res:?}"
        );
        assert_eq!(
            res.unwrap(),
            expected_provides,
            "check the provides variables are as expected"
        )
    }

    #[tokio::test]
    async fn run_environment_setup_provides_invalid_json_output() {
        let temp = TempDir::new().unwrap();
        let mut ctx = Context::new();

        let expected_err = "Environment setup output not valid json: \"some invalid json\\n\"";

        let script = indoc!(
            r#"
            #!/usr/bin/env sh
            echo "some invalid json" >> "$RTF_OUTPUT"
            "#
        );
        let test_plan = environment_setup_provides(script);

        let res = test_plan.run_environment_setup(&temp, &mut ctx).await;
        assert!(res.is_err(), "expected a json error, got {res:?}");
        assert_eq!(res.unwrap_err().to_string(), expected_err);
    }

    #[tokio::test]
    async fn run_environment_setup_provides_missing_variables() {
        let temp = TempDir::new().unwrap();
        let mut ctx = Context::new();

        let expected_err = r#"Missing required output fields from environment setup: ["bar"]"#;

        let script = indoc!(
            r#"
            #!/usr/bin/env sh
            echo "{ \"foo\": \"foo\" }" >> "$RTF_OUTPUT"
            "#
        );
        let test_plan = environment_setup_provides(script);

        let res = test_plan.run_environment_setup(&temp, &mut ctx).await;
        assert!(
            res.is_err(),
            "expected a missing variables error, got {res:?}"
        );
        assert_eq!(res.unwrap_err().to_string(), expected_err);
    }
}
