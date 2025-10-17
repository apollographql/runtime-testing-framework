use crate::{
    ValueDefinition,
    checks::{self, Check, CheckArrayDuplicates},
    context::ResolutionContext,
    formats::{EnvironmentConfig, Error, Matrix, Result, ScenarioConfig},
    merge_yaml,
    providers::{
        command::CommandSection,
        file::{RawSource, Source},
    },
    templating::{self, Scalar, Template},
};
use rtf_core::github::{self, Client};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use tracing::error;

/// The format for parsing scenario config
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TestPlanConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub values: HashMap<String, Scalar>,
    #[serde(default)]
    pub matrix: Matrix,
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
        let tp_source = Source::local(abs_path);

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
        let tp_source = Source::github(org, repo, path, git_ref);

        raw.try_into_test_plan(tp_source, ctx).await
    }

    /// Iteratate over all variants of this test plan that arise from [expanding](Matrix::expand)
    /// any matrix values that it contains.
    ///
    /// This will always return at least the base test plan itself if there are no matrix values
    /// defined.
    pub fn try_iter_matrix_variants(&self) -> Result<impl Iterator<Item = (String, Self)>> {
        let expanded = self.matrix.try_expand(&self.values)?;

        Ok(expanded.into_iter().map(|(name, values)| {
            let mut new = self.clone();
            new.values = values;
            new.matrix.clear();

            (name, new)
        }))
    }

    /// The set of allowed templating values that this test plan supports.
    ///
    /// This is the union of values defined as a scalars and those that are part of a matrix
    fn allowed_values(&self) -> HashSet<&String> {
        self.values.keys().chain(self.matrix.keys()).collect()
    }

    pub fn check_templating_will_work(&mut self) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        self.matrix.check_conflicting_keys(&self.values, &mut errs);
        self.matrix.check_dimensions(&mut errs);
        self.check_required_values(&mut errs);

        errs.into_result(())
    }

    /// Check that all required values have been defined somewhere within the test plan
    fn check_required_values(&self, errs: &mut templating::ErrorBuilder) {
        let mut check_missing_values =
            |section: &CommandSection,
             allowed: &HashSet<&String>,
             value_defs: &[ValueDefinition],
             error_path: &[String]| {
                let mut missing_values: Vec<String> = section
                    .required_values()
                    .iter()
                    .filter(|s| {
                        !(allowed.contains(*s)
                            || value_defs
                                .iter()
                                .any(|vd| &vd.name == *s && vd.default.is_some()))
                    })
                    .map(|val| {
                        format!(
                            "  - {val}: {:?}",
                            value_defs
                                .iter()
                                .find(|vd| &vd.name == val)
                                .map(|vd| vd.description.as_str())
                                .unwrap_or_default()
                        )
                    })
                    .collect();

                if !missing_values.is_empty() {
                    missing_values.sort_unstable(); // ensure consistent ordering
                    errs.push(
                        templating::ErrorKind::MissingValues,
                        missing_values.join("\n"),
                        error_path,
                    )
                }
            };

        let mut allowed_values = self.allowed_values();

        check_missing_values(
            &self.environment.setup.command,
            &allowed_values,
            &self.environment.values,
            &["environment".to_string(), "setup".to_string()],
        );

        // The scenario and teardown are permitted to use values coming from setup.provides in
        // addition to the values declared in the test plan itself
        allowed_values.extend(self.environment.setup.provides.iter().map(|val| &val.name));

        check_missing_values(
            &self.environment.teardown,
            &allowed_values,
            &self.environment.values,
            &["environment".to_string(), "teardown".to_string()],
        );

        check_missing_values(
            &self.scenario.command,
            &allowed_values,
            &self.scenario.values,
            &["scenario".to_string()],
        );
    }

    pub fn try_template_environment_setup(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut path = vec!["environment".to_string()];
        self.environment.try_template_setup(&mut path, values)
    }

    pub fn try_template_environment_teardown(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut path = vec!["environment".to_string()];
        self.environment.try_template_teardown(&mut path, values)
    }

    pub fn try_template_scenario(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut path = vec!["scenario".to_string()];
        self.scenario.try_template(&mut path, values)
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
            .run_providers_and_execute_for_output(out_dir, self.sources.environment(), ctx)
            .await?;

        let provides: HashMap<String, Scalar> = serde_json::from_str(&raw_output)?;

        let mut missing = Vec::new();
        for val in self.environment.setup.provides.iter() {
            if !provides.contains_key(&val.name) {
                missing.push(val.name.clone());
            }
        }

        if missing.is_empty() {
            Ok(provides)
        } else {
            Err(Error::InvalidSetupOutput { missing })
        }
    }

    pub async fn run_environment_teardown(
        &self,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> Result<()> {
        self.environment
            .teardown
            .run_providers_and_execute_for_output(out_dir, self.sources.environment(), ctx)
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
            .run_providers_and_execute_for_output(out_dir, self.sources.scenario(), ctx)
            .await?;

        Ok(())
    }

    /// Create an empty [TestPlanConfig] for tests
    #[cfg(test)]
    pub(crate) fn empty() -> TestPlanConfig {
        TestPlanConfig {
            name: Default::default(),
            description: Default::default(),
            values: Default::default(),
            matrix: Default::default(),
            scenario: ScenarioConfig::empty(),
            environment: EnvironmentConfig::empty(),
            sources: Sources {
                test_plan: Source::Local {
                    abs_path: Default::default(),
                },
                scenario: Default::default(),
                environment: Default::default(),
            },
        }
    }
}

impl Template for TestPlanConfig {
    fn has_pending_fields(&self) -> bool {
        self.environment.has_pending_fields() || self.scenario.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        let mut vals = self.environment.required_values();
        vals.extend(self.scenario.required_values());

        vals
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.environment.try_template_nested(
            path,
            "environment",
            values,
        ));
        errs.append(self.scenario.try_template_nested(path, "scenario", values));

        errs.into_result(())
    }
}

impl Check for TestPlanConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::from(self.environment.try_check_nested(
            path,
            "environment",
            src,
            ctx,
        ));
        errs.append(self.scenario.try_check_nested(path, "scenario", src, ctx));

        errs.into_result(())
    }
}

/// In order to be able to resolve relative paths we need to track where we obtained each of the
/// config files associated with a [TestPlan][TestPlanConfig].
///
/// If the [ScenarioConfig] or [EnvironmentConfig] are specified inline then their source will
/// match that of the overall [TestPlanConfig], otherwise we store the source as defined in the
/// [RawTestPlanConfig].
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct Sources {
    test_plan: Source,
    scenario: Option<Source>,
    environment: Option<Source>,
}

impl Sources {
    fn new(test_plan: Source, scenario: Option<Source>, environment: Option<Source>) -> Self {
        Self {
            test_plan,
            scenario,
            environment,
        }
    }

    pub fn test_plan(&self) -> &Source {
        &self.test_plan
    }

    /// The [Source] of the [EnvironmentConfig] in this test plan.
    ///
    /// Defaults to the source of the test plan itself if the environment was specified inline.
    pub fn environment(&self) -> &Source {
        match self.environment.as_ref() {
            Some(source) => source,
            None => &self.test_plan,
        }
    }

    /// The [Source] of the [ScenarioConfig] in this test plan.
    ///
    /// Defaults to the source of the test plan itself if the scenario was specified inline.
    pub fn scenario(&self) -> &Source {
        match self.scenario.as_ref() {
            Some(source) => source,
            None => &self.test_plan,
        }
    }
}

/// The raw format for parsing scenario config. This allows the [ScenarioConfig]
/// and [EnvironmentConfig] to be retrieved from files or inline content before
/// being fully templated and checked.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[schemars(title = "Test Plan Config")]
#[schemars(description = "The top level config file for specifying an RTF test plan")]
pub struct RawTestPlanConfig {
    /// The name of this test plan
    pub name: String,
    /// A brief description of the purpose / behaviour of this test plan
    pub description: String,
    /// Templating values to apply to fields within the rest of the test plan
    #[serde(default)]
    pub values: HashMap<String, Scalar>,
    /// Sets of templating values to apply to fields within the rest of the test plan as a matrix
    #[serde(default)]
    pub matrix: RawMatrix,
    /// The test scenario to execute
    pub scenario: ConfigSpec,
    /// The environment setup and teardown to run around the test scenario
    pub environment: ConfigSpec,
}

impl RawTestPlanConfig {
    async fn try_into_test_plan(
        self,
        tp_source: Source,
        ctx: &impl ResolutionContext,
    ) -> Result<TestPlanConfig> {
        let res = self
            .environment
            .try_into_config_with_source::<EnvironmentConfig>(&tp_source, ctx)
            .await;
        let (environment, environment_source) = match res {
            Ok(data) => data,
            Err(err) => {
                error!("malformed environment config section");
                return Err(err);
            }
        };

        let res = self
            .scenario
            .try_into_config_with_source::<ScenarioConfig>(&tp_source, ctx)
            .await;
        let (scenario, scenario_source) = match res {
            Ok(data) => data,
            Err(err) => {
                error!("malformed scenario config section");
                return Err(err);
            }
        };

        Ok(TestPlanConfig {
            name: self.name,
            description: self.description,
            values: self.values,
            matrix: self.matrix.into(),
            scenario,
            environment,
            sources: Sources::new(tp_source, scenario_source, environment_source),
        })
    }
}

/// We support only providing dimensions at the top level if the user doesn't care about
/// customising matrix variant names or using more advanced features.
///
/// This enum is only used to upgrade raw dimensions into a [Matrix] when parsing test plans.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum RawMatrix {
    Full(Matrix),
    Dimensions(HashMap<String, Vec<Scalar>>),
}

impl Default for RawMatrix {
    fn default() -> Self {
        Self::Full(Matrix::default())
    }
}

impl From<RawMatrix> for Matrix {
    fn from(m: RawMatrix) -> Self {
        match m {
            RawMatrix::Full(m) => m,
            RawMatrix::Dimensions(dimensions) => Matrix {
                variant_names: None,
                dimensions,
            },
        }
    }
}

/// # Config Spec
///
/// This struct allows the config files to be resolved either from
/// inline yaml in the test plan or from a `FileProvider`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(
    untagged,
    expecting = "expected valid inline config section or from with overrides"
)]
pub enum ConfigSpec {
    Inline {
        /// Inline YAML configuration
        #[schemars(schema_with = "arbitrary_map")]
        inline: serde_yaml::Value,
    },
    From {
        /// A source file to read to obtain the required configuration
        from: RawSource,
        /// Optional overrides to merge on top of the base configuration
        #[serde(default)]
        #[schemars(schema_with = "arbitrary_map")]
        overrides: serde_yaml::Value,
    },
}

fn arbitrary_map(_gen: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "object" })
}

impl ConfigSpec {
    /// Try to convert the [ConfigSpec] into a config yaml. This can read the content
    /// directly from inline content or a [FileProvider]
    async fn try_into_config_with_source<T>(
        self,
        tp_source: &Source,
        ctx: &impl ResolutionContext,
    ) -> Result<(T, Option<Source>)>
    where
        T: CheckArrayDuplicates + DeserializeOwned,
    {
        match self {
            Self::Inline { inline } => {
                let mut t: T = serde_yaml::from_value(inline)?;
                t.ensure_no_duplicate_keys()?;
                t.sort_arrays();

                Ok((t, None))
            }
            Self::From {
                from,
                mut overrides,
            } => {
                // When applying overrides we need to make sure that the base config file is valid
                // before we start and then re-validate following the merge.
                let src = from.try_into_source(tp_source, ctx)?;
                let file_content = src.try_get_file_content(ctx).await?;
                let mut t: T = serde_yaml::from_str(&file_content)?;
                t.ensure_no_duplicate_keys()?;

                if overrides != serde_yaml::Value::Null {
                    let mut base: serde_yaml::Value = serde_yaml::from_str(&file_content)?;
                    let yaml_src = serde_yaml::to_value(tp_source)?;
                    set_source_for_relative_files(&mut overrides, &yaml_src);
                    merge_yaml(overrides, &mut base);

                    // Now that we've merged we need to handle deduplication of arrays in order to
                    // retain any data from the overrides in favour of what was in the base config
                    // file. try_dedup_and_sort can error at this stage if there were duplicates
                    // within the overrides themselves.
                    let mut merged: T = serde_yaml::from_value(base)?;
                    merged.try_dedup_and_sort()?;
                    t = merged;
                }

                Ok((t, Some(src)))
            }
        }
    }
}

/// Relative files set as part of overrides need to be resolved relative to the test plan source
/// location rather than the source location of the file they are being merged into.
///
/// To support this we tag any RelativeFile file providers we can find with the source of the test
/// plan before we merge _at the YAML level_. We do it this way to avoid having to define Rust
/// types for the overrides where every field is optional, but this does mean that we have zero
/// type safety around this.
///
/// !! If something strange is happening around relative paths defined in test plan overrides then
///    this is likely the best place to start looking!
fn set_source_for_relative_files(val: &mut serde_yaml::Value, src: &serde_yaml::Value) {
    use serde_yaml::Value;

    match val {
        Value::Mapping(map) => {
            if map.get("kind").and_then(|v| v.as_str()) == Some("relative_path") {
                map.insert(Value::String("src".into()), src.clone());
                return;
            }

            for v in map.values_mut() {
                set_source_for_relative_files(v, src);
            }
        }

        Value::Sequence(seq) => {
            for v in seq {
                set_source_for_relative_files(v, src);
            }
        }

        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        formats::{
            environment::{
                SetupSection,
                test_helpers::{environment_with_fields, templatable_environment},
            },
            scenario::test_helpers::{scenario_with_fields, templatable_scenario},
            tests::{
                assert_check_errors, assert_template_errors, expected_error_details,
                named_file_provider_with_field, p, r, templatable_file_providers,
                value_definitions, value_map,
            },
        },
        providers::{
            command::{
                CommandProvider, CommandSection, CommandSpec,
                test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            },
            file::{FileProvider, InlineFile, NamedFileProvider},
        },
        templating::{ErrorBuilder, ErrorKind, Field},
    };
    use assert_fs::{
        TempDir,
        prelude::{FileWriteStr, PathChild},
    };
    use indoc::indoc;
    use simple_test_case::test_case;

    // Helper functions

    /// Create a ValueDefinition with a default value
    fn value_with_default(name: &str, val: &str) -> ValueDefinition {
        ValueDefinition {
            name: name.into(),
            description: String::default(),
            default: Some(val.into()),
        }
    }

    /// Create a HashMap of values from key-value pairs using try_from
    macro_rules! values_map {
        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();
            $( m.insert($k.to_string(), $crate::templating::Scalar::try_from($v).unwrap()); )+
            m
        }};
    }

    /// Create a TestPlanConfig for testing Template trait methods (has_pending_fields, required_values)
    fn test_plan_with_fields(
        scenario_fields: &[Field<String>],
        environment_fields: &[Field<String>],
    ) -> TestPlanConfig {
        TestPlanConfig {
            scenario: scenario_with_fields(scenario_fields),
            environment: environment_with_fields(&[], environment_fields),
            ..TestPlanConfig::empty()
        }
    }

    /// Create a TestPlanConfig for template tests
    fn templatable_test_plan(
        values: HashMap<String, Scalar>,
        matrix: HashMap<String, Vec<Scalar>>,
        scenario_fields: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
    ) -> TestPlanConfig {
        let mut env_values = setup_fields.to_vec();
        env_values.extend_from_slice(teardown_fields);

        TestPlanConfig {
            values,
            matrix: Matrix {
                variant_names: None,
                dimensions: matrix,
            },
            scenario: templatable_scenario(scenario_fields, scenario_fields),
            environment: templatable_environment(&env_values, setup_fields, teardown_fields),
            ..TestPlanConfig::empty()
        }
    }

    /// Create a matrix with two values per key provided
    fn matrix_from_keys(keys: &[&str], no_entries: isize) -> HashMap<String, Vec<Scalar>> {
        keys.iter()
            .map(|&key| {
                (
                    key.to_string(),
                    if no_entries <= 0 {
                        Vec::new()
                    } else {
                        (0..no_entries)
                            .map(|i| format!("{key}{}", i + 1).into())
                            .collect()
                    },
                )
            })
            .collect()
    }

    // Tests for configuration parsing from inline YAML and external files

    const INLINE_TEST_PLAN: &str = indoc!(
        r#"
            name: inline-test-plan
            description: test plan with inline scenario and environment
            values:
              foo: "foo"
              bar: "bar"
            scenario:
              inline: 
                name: inline-scenario
                description: an inline scenario
                values:
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
                values:
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
    async fn parse_and_template_inline_test_plan_config() {
        let raw_test_plan: RawTestPlanConfig =
            serde_yaml::from_str(INLINE_TEST_PLAN).expect("test plan config to parse");
        let ctx = Context::new();

        let expected_sources = Sources {
            test_plan: Source::Local {
                abs_path: "/".into(),
            },
            scenario: None,
            environment: None,
        };

        let res = raw_test_plan
            .try_into_test_plan(
                Source::Local {
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

        let res = test_plan.required_values();
        assert_eq!(
            res,
            &["bar", "baz", "foo"],
            "check that test plan returns fields"
        )
    }

    const FROM_SAME_DIR_FILES_TEST_PLAN: &str = indoc!(
        r#"
            name: from-files-test-plan
            description: test plan with scenario and environment from files
            values:
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
            values:
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

        // Build expected sources using the same canonicalization as the implementation
        let expected_sources = Sources {
            test_plan: Source::Local {
                abs_path: ctx.canonicalize_path(&tp_file).unwrap(),
            },
            scenario: Some(Source::Local {
                abs_path: ctx.canonicalize_path(&scenario_file).unwrap(),
            }),
            environment: Some(Source::Local {
                abs_path: ctx.canonicalize_path(&environment_file).unwrap(),
            }),
        };

        let res = TestPlanConfig::try_load_and_resolve_from_path(tp_file.to_path_buf(), &ctx).await;
        assert!(res.is_ok(), "expected TestPlanConfig, got {res:?}");

        let test_plan = res.unwrap();
        let sources = test_plan.clone().sources;
        assert_eq!(
            sources, expected_sources,
            "test that sources are set correctly"
        );

        let res = test_plan.required_values();
        let expected_values: &[&str] = &[];
        assert_eq!(res, expected_values, "check that test plan returns fields")
    }

    const OVERRIDES_TEST_PLAN: &str = indoc!(
        r#"
            name: from-files-test-plan
            description: test plan with scenario and environment from files
            values:
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
    async fn parse_test_plan_with_overrides() {
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

        let res = test_plan.required_values();
        let expected_values: &[&str] = &[];
        assert_eq!(res, expected_values, "check that test plan returns fields")
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
        let test_plan = test_plan_with_fields(&[scenario_field], &[environment_field]);

        let res = test_plan.has_pending_fields();
        assert_eq!(
            res, expected,
            "tests that has_pending_values has expected value"
        )
    }

    #[test_case(p("scenario"), p("environment"), &["environment", "scenario"]; "scenario and environment fields required")]
    #[test_case(p("scenario"), r("environment"), &["scenario"]; "scenario field required")]
    #[test_case(r("scenario"), p("environment"), &["environment"]; "environment field required")]
    #[test_case(r("scenario"), r("environment"), &[]; "no fields required")]
    #[test]
    fn required_values(
        scenario_field: Field<String>,
        environment_field: Field<String>,
        expected: &[&str],
    ) {
        let test_plan = test_plan_with_fields(&[scenario_field], &[environment_field]);

        let res = test_plan.required_values();
        assert_eq!(
            res, expected,
            "tests that required_values has expected value"
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
    #[test_case(&["foo", "bar", "baz"], &[], &[]; "scenario multi value and no environment")]
    #[test_case(&[], &["foo", "bar"], &[]; "setup multi value and no teardown")]
    #[test_case(&[], &[], &["foo", "bar"]; "teardown multi value and no setup")]
    #[test_case(&["s1", "s2"], &["setup1", "setup2"], &[]; "scenario and setup multi value")]
    #[test_case(&["s1", "s2"], &[], &["teardown1", "teardown2"]; "scenario and teardown multi value")]
    #[test_case(&[], &["setup1", "setup2"], &["teardown1", "teardown2"]; "setup and teardown multi value")]
    #[test_case(&["s1", "s2"], &["setup1", "setup2"], &["teardown1", "teardown2"]; "all sections multi value")]
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
                values: value_definitions(scenario_fields),
                command: CommandSection {
                    file_providers: templatable_file_providers(scenario_fields),
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            environment: EnvironmentConfig {
                values: value_definitions(env_fields.as_slice()),
                teardown: CommandSection {
                    file_providers: templatable_file_providers(env_fields.as_slice()),
                    ..CommandSection::empty()
                },
                ..EnvironmentConfig::empty()
            },
            ..TestPlanConfig::empty()
        };
        let result = test_plan.try_template(&mut Vec::new(), &value_map(all_fields.as_slice()));

        assert!(
            result.is_ok(),
            "expected templating to succeed, got {:?}",
            result
        );
    }

    /// Helper for creating a test plan for Template tests
    fn template_test_plan(
        scenario_value_defs: &[&str],
        scenario_fields: &[&str],
        env_value_defs: &[&str],
        env_fields: &[&str],
    ) -> TestPlanConfig {
        TestPlanConfig {
            scenario: ScenarioConfig {
                values: value_definitions(scenario_value_defs),
                command: CommandSection {
                    file_providers: templatable_file_providers(scenario_fields),
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            environment: EnvironmentConfig {
                values: value_definitions(env_value_defs),
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
        values: HashMap<String, Scalar>,
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

        assert_template_errors(test_plan, values, expected_err_messages, expected_err_paths);
    }

    #[test_case(&["missing"], &["scenario"], &["scenario"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["scenario1", "scenario2"], &["scenario1", "scenario2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["scenario1", "missing2"], &["scenario1", "scenario2"], &["scenario2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but value not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but values not provided")]
    #[test]
    fn try_template_scenario_missing_value_definitions(
        value_defs: &[&str],
        fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let values = value_map(&["scenario", "scenario1", "scenario2"]);
        let mut test_plan = template_test_plan(value_defs, fields, &[], &[]);

        assert_test_plan_template_errors(&mut test_plan, values, expected_err_fields, &[]);
    }

    #[test_case(&["missing"], &["environment"], &["environment"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["environment1", "environment2"], &["environment1", "environment2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["environment1", "missing2"], &["environment1", "environment2"], &["environment2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but value not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but values not provided")]
    #[test]
    fn try_template_environment_missing_value_definitions(
        value_defs: &[&str],
        fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let values = value_map(&["environment", "environment1", "environment2"]);
        let mut test_plan = template_test_plan(&[], &[], value_defs, fields);

        assert_test_plan_template_errors(&mut test_plan, values, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_scenario_and_environment_value_definitions() {
        let values = value_map(&["scenario", "environment"]);
        let mut test_plan = template_test_plan(&[], &["scenario"], &[], &["environment"]);

        assert_test_plan_template_errors(&mut test_plan, values, &["scenario"], &["environment"]);
    }

    #[test]
    fn try_template_missing_scenario_and_environment_values_not_provided() {
        let values = value_map(&[]);
        let mut test_plan = template_test_plan(
            &["scenario"],
            &["scenario"],
            &["environment"],
            &["environment"],
        );

        assert_test_plan_template_errors(&mut test_plan, values, &["scenario"], &["environment"]);
    }

    // Tests for matrix expansion, variants, and matrix-related functionality

    #[test_case(
        &[],
        &[],
        &[
            HashMap::new()
        ];
        "empty matrix and empty values"
    )]
    #[test_case(
        &["foo", "bar"],
        &[],
        &[
            values_map!("foo" => "foo", "bar" => "bar")
        ];
        "empty matrix with values"
    )]
    #[test_case(
        &[],
        &[("key", vec!["a", "b", "c"])],
        &[
            values_map!("key" => "a"),
            values_map!("key" => "b"), 
            values_map!("key" => "c")
        ];
        "single key matrix with multiple entries and no values"
    )]
    #[test_case(
        &["foo"],
        &[("key", vec!["a", "b", "c"])],
        &[
            values_map!("foo" => "foo", "key" => "a"),
            values_map!("foo" => "foo", "key" => "b"), 
            values_map!("foo" => "foo", "key" => "c")
        ];
        "single key matrix with multiple entries and one value"
    )]
    #[test_case(
        &[],
        &[("key1", vec!["a", "b", "c"]), ("key2", vec!["1", "2"])],
        &[
            values_map!("key1" => "a", "key2" => "1"),
            values_map!("key1" => "a", "key2" => "2"),
            values_map!("key1" => "b", "key2" => "1"),
            values_map!("key1" => "b", "key2" => "2"),
            values_map!("key1" => "c", "key2" => "1"),
            values_map!("key1" => "c", "key2" => "2")
        ];
        "multiple keys with multiple entries and no values"
    )]
    #[test]
    fn matrix_expansion(
        values: &[&str],
        matrix: &[(&str, Vec<&str>)],
        expected_values_maps: &[HashMap<String, Scalar>],
    ) {
        let values = value_map(values);
        let dimensions: HashMap<String, Vec<Scalar>> = matrix
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| Scalar::from(*s)).collect()))
            .collect();
        let test_plan = TestPlanConfig {
            values: values.clone(),
            matrix: Matrix {
                variant_names: None,
                dimensions,
            },
            ..TestPlanConfig::empty()
        };

        let variants: Vec<_> = test_plan.try_iter_matrix_variants().unwrap().collect();
        assert_eq!(
            variants.len(),
            expected_values_maps.len(),
            "test the variants from iter_matrix_variants has the xepcted combination count"
        );
        assert!(
            variants.iter().all(|(_, v)| v.matrix.is_empty()),
            "expected all variants to have an empty matrix"
        );

        let n_variants = test_plan.matrix.n_variants();
        assert_eq!(
            n_variants,
            expected_values_maps.len(),
            "test that the number of variants generated using iter_matrix_variants matches n_matrix_variants"
        );

        // Get the expanded matrix values to make sure this outputs the same values as iter_matrix_variants
        let expanded_matrix_values = test_plan
            .matrix
            .try_expand(&test_plan.values)
            .expect("expansion to succeed");

        // Check each variant has the expected combinations in the order expected
        for (i, (_, variant)) in variants.iter().enumerate() {
            let expected_values = expected_values_maps[i].clone();
            let expanded_values = expanded_matrix_values[i].1.clone();

            assert_eq!(
                variant.values, expected_values,
                "test the combination matches the expected one"
            );
            assert_eq!(
                variant.values, expanded_values,
                "test the combination from iter_matrix_variants matches the combination in expanded_matrix_variants"
            );
        }
    }

    // Tests for try_templating_will_work and its dependent functions
    #[test_case(&["scenario", "setup", "teardown"], &[]; "fields for scenario setup and teardown in values")]
    #[test_case(&["scenario", "setup"], &["teardown"]; "fields for scenario and setup from values and teardown from matrix")]
    #[test_case(&["scenario", "teardown"], &["setup"]; "fields for scenario and teardown from values and setup from matrix")]
    #[test_case(&["scenario"], &["setup", "teardown"]; "fields for scenario from values and setup and teardown from matrix")]
    #[test_case(&["setup"], &["scenario", "teardown"]; "fields for setup from values and scenario and teardown from matrix")]
    #[test_case(&["setup", "teardown"], &["scenario"]; "fields for setup and teardown from values and scenario from matrix")]
    #[test_case(&["teardown"], &["scenario", "setup"]; "fields for teardown from values and scenario and setup from matrix")]
    #[test_case(&[], &["scenario", "setup", "teardown"]; "fields for scenario setup and teardown in matrix")]
    #[test]
    fn check_templating_will_work_success(value_keys: &[&str], matrix_keys: &[&str]) {
        let values = value_map(value_keys);
        let matrix = matrix_from_keys(matrix_keys, 1);
        let mut test_plan =
            templatable_test_plan(values, matrix, &["scenario"], &["setup"], &["teardown"]);

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test_case(&["foo"], "foo"; "single conflicting key")]
    #[test]
    #[test_case(&["foo", "bar", "baz"], "bar, baz, foo"; "multiple conflicting keys")]
    fn check_templating_will_work_conflicting_keys_errors(
        matrix_keys: &[&str],
        expected_err_message: &str,
    ) {
        let values = value_map(&["foo", "bar", "baz"]);
        let matrix = matrix_from_keys(matrix_keys, 2);
        let mut test_plan = templatable_test_plan(values, matrix, &[], &[], &[]);

        let expected_err_kind = ErrorKind::ConflictingValues;

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let error = res.unwrap_err().unwrap_single();
        assert_eq!(
            error.kind, expected_err_kind,
            "test that the error kind is as expected"
        );
        assert_eq!(
            error.message, expected_err_message,
            "test that error message is as expected"
        );
    }

    #[test_case(&["foo"], &["foo"]; "single matrix")]
    #[test_case(&["foo", "bar", "baz"], &["bar", "baz", "foo"]; "multiple matrices")]
    #[test]
    fn check_templating_will_work_empty_matrix_errors(
        matrix_keys: &[&str],
        expected_err_messages: &[&str],
    ) {
        let values = value_map(&[]);
        let matrix = matrix_from_keys(matrix_keys, 0);
        let mut test_plan = templatable_test_plan(values, matrix, &[], &[], &[]);

        let expected_err_kind = ErrorKind::EmptyMatrixValue;

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_err_messages.len(),
            "test that the expected number of errors occur"
        );
        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::EmptyMatrixValue)),
            "expected all errors to be {:?}, got {:?}",
            expected_err_kind,
            errors
        );
        let mut err_messages: Vec<String> = errors.iter().map(|f| f.message.clone()).collect();
        err_messages.sort();
        assert_eq!(
            err_messages, expected_err_messages,
            "test that the error messages are as expected"
        );
    }

    #[test]
    fn check_templating_will_work_inconsistent_matrix_value_errors() {
        let values = HashMap::new();
        let mut matrix: HashMap<String, Vec<Scalar>> = HashMap::new();
        matrix.insert("foo".into(), vec!["a".into(), 42.into()]);
        let mut test_plan = templatable_test_plan(values, matrix, &[], &[], &[]);

        let expected_err_kind = ErrorKind::InconsistentMatrixValue;
        let expected_err_message = "foo";

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let error = res.unwrap_err().unwrap_single();
        assert_eq!(
            error.kind, expected_err_kind,
            "test that the error kind is as expected"
        );
        assert_eq!(
            error.message, expected_err_message,
            "test that error message is as expected"
        );
    }

    #[test]
    fn check_required_values_setup_provides_available_to_scenario_and_teardown() {
        let provides = vec![ValueDefinition {
            name: "provides".to_string(),
            description: "A value provided by setup".to_string(),
            default: None,
        }];

        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                setup: SetupSection {
                    command: CommandSection {
                        ..CommandSection::empty()
                    },
                    provides,
                },
                teardown: CommandSection {
                    file_providers: vec![named_file_provider_with_field("foo", p("provides"))],
                    ..CommandSection::empty()
                },
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                command: CommandSection {
                    file_providers: vec![named_file_provider_with_field("foo", p("provides"))],
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test_case(&["scenario"], &[], &[], &["scenario"]; "scenario missing values")]
    #[test_case(&[], &["setup"], &[], &["setup"]; "setup missing values")]
    #[test_case(&[], &[], &["teardown"], &["teardown"]; "teardown missing values")]
    #[test_case(&[], &["setup"], &["teardown"], &["setup", "teardown"]; "setup and teardown missing values")]
    #[test_case(&["scenario"], &["setup"], &[], &["setup", "scenario"]; "scenario and setup missing values")]
    #[test_case(&["scenario"], &[], &["teardown"], &["teardown", "scenario"]; "scenario and teardown missing values")]
    #[test_case(&["scenario"], &["setup"], &["teardown"], &["setup", "teardown", "scenario"]; "scenario and setup and teardown missing values")]
    #[test]
    fn check_templating_will_work_missing_values_errors(
        scenario_fields: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        expected_err_messages: &[&str],
    ) {
        let values = value_map(&["foo"]);
        let matrix = HashMap::new();
        let mut test_plan = templatable_test_plan(
            values,
            matrix,
            scenario_fields,
            setup_fields,
            teardown_fields,
        );

        let expected_err_kind = ErrorKind::MissingValues;

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_err_messages.len(),
            "test that the expected number of errors occur"
        );

        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::MissingValues)),
            "expected all errors to be {:?}, got {:?}",
            expected_err_kind,
            errors
        );

        let error_messages: Vec<String> = errors.iter().map(|f| f.message.clone()).collect();
        let expected_err_messages: Vec<String> = expected_err_messages
            .iter()
            .map(|f| format!("  - {}: \"description\"", f))
            .collect();
        assert_eq!(
            error_messages, expected_err_messages,
            "test that the error messages are as expected"
        );
    }

    #[test]
    fn check_templating_will_work_combined_errors() {
        let values = value_map(&["foo", "bar"]);
        let matrix = matrix_from_keys(&["foo"], 0);
        let mut test_plan = templatable_test_plan(values, matrix, &["scenario"], &[], &[]);

        let mut expected_errs = ErrorBuilder::new();
        expected_errs.push(ErrorKind::ConflictingValues, "foo", &["".to_string()]);
        expected_errs.push(ErrorKind::EmptyMatrixValue, "foo", &["".to_string()]);
        expected_errs.push(
            ErrorKind::MissingValues,
            "  - scenario: \"description\"",
            &["scenario".to_string()],
        );
        let expected_errs = expected_errs.into_result("").unwrap_err();

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );
        assert_eq!(
            res.unwrap_err(),
            expected_errs,
            "test that combined errors are as expected"
        );
    }

    #[test]
    fn check_required_values_value_definition_defaults_count_as_required_values() {
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                values: vec![
                    value_with_default("setup", "setup"),
                    value_with_default("teardown", "teardown"),
                ],
                setup: SetupSection {
                    command: CommandSection {
                        file_providers: vec![named_file_provider_with_field("setup", p("setup"))],
                        ..CommandSection::empty()
                    },
                    provides: Vec::new(),
                },
                teardown: CommandSection {
                    file_providers: vec![named_file_provider_with_field("teardown", p("teardown"))],
                    ..CommandSection::empty()
                },
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                values: vec![value_with_default("scenario", "scenario")],
                command: CommandSection {
                    file_providers: vec![named_file_provider_with_field("scenario", p("scenario"))],
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work();
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test]
    fn try_check_success() {
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
        let src = Source::Local {
            abs_path: "/".into(),
        };

        let res = test_plan.try_check(&mut Vec::new(), &src, &ctx);
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
        let src = Source::Local {
            abs_path: "/".into(),
        };

        assert_check_errors(test_plan, &src, &ctx, expected_err_kinds);
    }
}
