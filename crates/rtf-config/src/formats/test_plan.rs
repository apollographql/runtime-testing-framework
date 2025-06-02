use crate::{
    context::{Context, ResolutionContext},
    formats::{EnvironmentConfig, Result, ScenarioConfig},
    providers::{
        self,
        file::{AsUtf8FileContent, FileProvider},
    },
    templating::{self, Scalar, Template},
    validation::{self, Validate},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::HashMap, fs, hash::Hash, mem::take, path::Path};

/// The format for parsing scenario config
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TestPlanConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub values: HashMap<String, Scalar>,
    pub scenario: ScenarioConfig,
    pub environment: EnvironmentConfig,
}

impl TestPlanConfig {
    pub async fn try_load_and_resolve_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p.as_ref())?;
        let raw: RawTestPlanConfig = serde_yaml::from_str(&content)?;

        let full_path = p.as_ref().canonicalize()?;
        let ctx = Context::new(full_path.parent().unwrap());

        raw.try_into_test_plan(&ctx).await
    }

    pub fn validate_templating_will_work(&mut self) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        let missing_env_setup_values: Vec<_> = self
            .environment
            .setup
            .command
            .required_values()
            .into_iter()
            .filter(|s| !self.values.keys().any(|v| v == s))
            .collect();

        let env_setup_is_missing_values = { !missing_env_setup_values.is_empty() };
        if env_setup_is_missing_values {
            errs.push(
                templating::ErrorKind::MissingValues,
                "Environment setup is missing values required for templating",
                &Vec::<String>::new(),
            )
        }

        let provides_values = &self.environment.setup.provides;
        let provides_values_keys: Vec<String> =
            provides_values.iter().map(|v| v.name.clone()).collect();

        let mut combined_keys: Vec<String> = self.values.keys().cloned().collect();
        combined_keys.extend(provides_values_keys);

        let missing_env_teardown_values: Vec<_> = self
            .environment
            .teardown
            .required_values()
            .into_iter()
            .filter(|s| !combined_keys.iter().any(|v| v == s))
            .collect();

        let env_teardown_is_missing_values = { !missing_env_teardown_values.is_empty() };
        if env_teardown_is_missing_values {
            errs.push(
                templating::ErrorKind::MissingValues,
                "Environment teardown is missing values required for templating",
                &Vec::<String>::new(),
            )
        }

        let missing_scenario_values: Vec<_> = self
            .scenario
            .required_values()
            .into_iter()
            .filter(|s| !combined_keys.iter().any(|v| v == s))
            .collect();

        let scenario_is_missing_values = { !missing_scenario_values.is_empty() };
        if scenario_is_missing_values {
            errs.push(
                templating::ErrorKind::MissingValues,
                "Scenario is missing values required for templating",
                &Vec::<String>::new(),
            )
        }

        errs.into_result(())
    }

    pub fn try_resolve_envrionment_setup(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        let mut path = vec!["environment".to_string()];

        if let Err(e) = self.environment.try_resolve_setup(&mut path, values) {
            errs.extend(e);
        };

        debug_assert!(
            !self.environment.setup.command.has_pending_fields(),
            "Envrionment setup should not have pending fields"
        );

        errs.into_result(())
    }

    pub fn try_resolve_envrionment_teardown(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        let mut path = vec!["environment".to_string()];

        if let Err(e) = self.environment.try_resolve_teardown(&mut path, values) {
            errs.extend(e);
        };

        debug_assert!(
            !self.environment.teardown.has_pending_fields(),
            "Envrionment teardown should not have pending fields"
        );

        errs.into_result(())
    }

    pub fn try_resolve_scenario(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        let mut path = vec!["scenario".to_string()];

        if let Err(e) = self.scenario.try_resolve(&mut path, values) {
            errs.extend(e);
        };

        debug_assert!(
            !self.scenario.has_pending_fields(),
            "Scenario should not have pending fields"
        );

        errs.into_result(())
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

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, crate::templating::Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        if let Err(e) = self
            .environment
            .try_resolve_nested(path, "environment", values)
        {
            errs.extend(e);
        };
        if let Err(e) = self.scenario.try_resolve_nested(path, "scenario", values) {
            errs.extend(e);
        };

        errs.into_result(())
    }
}

impl Validate for TestPlanConfig {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> crate::validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        if let Err(e) = self
            .environment
            .try_validate_nested(path, "environent", ctx)
        {
            errs.extend(e);
        };

        if let Err(e) = self.scenario.try_validate_nested(path, "scenario", ctx) {
            errs.extend(e);
        };

        errs.into_result(())
    }
}

/// The raw format for parsing scenario config. This allows the [ScenarioConfig]
/// and [EnvironmentConfig] to be retrieved from files or inline content before
/// being fully templated and validated.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RawTestPlanConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub values: HashMap<String, Scalar>,
    pub scenario: ConfigSource<ScenarioConfig>,
    pub environment: ConfigSource<EnvironmentConfig>,
}

impl RawTestPlanConfig {
    async fn try_into_test_plan(self, ctx: &impl ResolutionContext) -> Result<TestPlanConfig> {
        let environment_source = self.environment;
        let mut environment = environment_source.try_into_config(ctx).await?;

        let scenario_source = self.scenario;
        let mut scenario = scenario_source.try_into_config(ctx).await?;

        dedup_and_sort_by_key(&mut environment.values, |v| v.name.clone());
        dedup_and_sort_by_key(&mut environment.setup.provides, |v| v.name.clone());
        dedup_and_sort_by_key(&mut environment.setup.command.file_providers, |nfp| {
            nfp.env_var.clone()
        });
        dedup_and_sort_by_key(&mut environment.teardown.file_providers, |nfp| {
            nfp.env_var.clone()
        });
        dedup_and_sort_by_key(&mut scenario.values, |v| v.name.clone());
        dedup_and_sort_by_key(&mut scenario.output_directories, |s| s.clone());
        dedup_and_sort_by_key(&mut scenario.command.file_providers, |nfp| {
            nfp.env_var.clone()
        });

        Ok(TestPlanConfig {
            name: self.name,
            description: self.description,
            values: self.values,
            environment,
            scenario,
        })
    }
}

/// Helper function for deduplicating list entries. We are depulicating in this
/// way because we know the user is overriding specific items. We are taking
/// the last entry on the list as the one to keep.
fn dedup_and_sort_by_key<T, K>(v: &mut Vec<T>, key_fn: fn(&T) -> K)
where
    K: Eq + Hash + Ord,
{
    let mut m = HashMap::with_capacity(v.len());
    for item in take(v).into_iter() {
        m.insert(key_fn(&item), item);
    }
    let mut deduped: Vec<T> = m.into_values().collect();
    deduped.sort_by_key(key_fn);

    *v = deduped;
}

/// This struct allows the config files to be resolved either from
/// inline yaml in the test plan or from a [FileProvider]
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ConfigSource<T> {
    Inline {
        inline: T,
    },
    From {
        from: FileProvider,
        #[serde(default)]
        overrides: serde_yaml::Value,
    },
}

impl<T> ConfigSource<T>
where
    T: DeserializeOwned,
{
    /// Try to convert the [ConfigSource] into a config yaml. This can read the content
    /// directly from inline content or a [FileProvider]
    async fn try_into_config(self, ctx: &impl ResolutionContext) -> providers::Result<T> {
        match self {
            Self::Inline { inline } => Ok(inline),
            Self::From { from, overrides } => {
                let file_content = from.try_get_file_content(ctx).await?;
                let mut base: serde_yaml::Value = serde_yaml::from_str(&file_content)?;
                if overrides != serde_yaml::Value::Null {
                    merge(overrides, &mut base);
                }

                Ok(serde_yaml::from_value(base)?)
            }
        }
    }
}

/// A function for merging yaml overrides with the base config. It is expected
/// behaviour that lists will be a combination of the base and override lists
/// for the same key. This function performs no deduplication.
fn merge(overrides: serde_yaml::Value, base: &mut serde_yaml::Value) {
    use serde_yaml::Value;

    match (overrides, base) {
        // If both values are mappings we add all keys from src into dst.
        (Value::Mapping(override_map), Value::Mapping(base_map)) => {
            for (key, override_val) in override_map.into_iter() {
                // If a key is present in both maps then we recursively merge the values,
                // otherwise we just insert the src key into dst directly.
                match base_map.get_mut(&key) {
                    Some(base_val) => merge(override_val, base_val),
                    None => _ = base_map.insert(key, override_val),
                };
            }
        }

        // If both values are sequences we append overrides to base
        (Value::Sequence(override_seq), Value::Sequence(base_seq)) => {
            base_seq.extend_from_slice(&override_seq)
        }

        // Otherwise we replace base with overrides
        (overrides, base) => *base = overrides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::Context, providers::file::InlineFile};
    use simple_test_case::dir_cases;
    use simple_txtar::Archive;
    use std::path::PathBuf;

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

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/valid")]
    #[tokio::test]
    async fn valid_config(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected: TestPlanConfig = serde_yaml::from_str(get_file(&arr, "expected")).unwrap();

        let raw_plan: RawTestPlanConfig = match serde_yaml::from_str(config) {
            Ok(plan) => plan,
            Err(e) => panic!("expected a valid RawTestPlanConfig, got: {e}"),
        };

        let ctx = Context::new(
            PathBuf::from("resources/config-tests/test-plan/valid")
                .canonicalize()
                .unwrap(),
        );

        let res = raw_plan.try_into_test_plan(&ctx).await;
        assert!(res.is_ok(), "expected a test plan config, got: {res:?}");

        let plan = res.unwrap();
        let res = plan.try_validate(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected test plan to validate, got {res:?}");
        pretty_assertions::assert_eq!(plan, expected, "expected test plan and expected to match");
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/valid-templates")]
    #[test]
    fn valid_templates_all(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let provides_values: HashMap<String, Scalar> =
            serde_yaml::from_str(get_file(&arr, "provides-values")).unwrap();
        let raw_expected = get_file(&arr, "after-templating");

        let mut plan_config: TestPlanConfig = serde_yaml::from_str(config).unwrap();
        let expected: TestPlanConfig = serde_yaml::from_str(raw_expected).unwrap();

        let mut combined_values = provides_values.clone();
        combined_values.extend(plan_config.values.clone());

        assert!(plan_config.has_pending_fields(), "fields should be pending");

        let res = plan_config.try_resolve(&mut Vec::new(), &combined_values);
        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(
            !plan_config.scenario.has_pending_fields(),
            "fields should be resolved"
        );
        assert!(
            !plan_config.has_pending_fields(),
            "fields should be resolved"
        );
        pretty_assertions::assert_eq!(plan_config, expected);
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/valid-templates")]
    #[test]
    fn valid_templates_partial(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let provides_values: HashMap<String, Scalar> =
            serde_yaml::from_str(get_file(&arr, "provides-values")).unwrap();
        let raw_expected = get_file(&arr, "after-templating");

        let mut plan_config: TestPlanConfig = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = plan_config.values.clone();
        let expected: TestPlanConfig = serde_yaml::from_str(raw_expected).unwrap();

        assert!(
            plan_config.validate_templating_will_work().is_ok(),
            "templating should work"
        );
        assert!(plan_config.has_pending_fields(), "fields should be pending");

        let res = plan_config.try_resolve_envrionment_setup(&values);
        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(
            !plan_config.environment.setup.command.has_pending_fields(),
            "fields should be resolved"
        );

        let mut combined_values = provides_values.clone();
        combined_values.extend(plan_config.values.clone());

        let res = plan_config.try_resolve_envrionment_teardown(&combined_values);
        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(
            !plan_config.environment.teardown.has_pending_fields(),
            "fields should be resolved"
        );

        let res = plan_config.try_resolve_scenario(&combined_values);
        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(
            !plan_config.scenario.has_pending_fields(),
            "fields should be resolved"
        );
        assert!(
            !plan_config.has_pending_fields(),
            "fields should be resolved"
        );
        pretty_assertions::assert_eq!(plan_config, expected);
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/scenario/parse-failures")]
    #[test]
    fn parse_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let res: serde_yaml::Result<TestPlanConfig> = serde_yaml::from_str(config);

        assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/invalid-templates")]
    #[test]
    fn invalid_templates(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "templating-errors");

        let mut plan_config: TestPlanConfig = serde_yaml::from_str(config).unwrap();
        let _values: HashMap<String, Scalar> = plan_config.values.clone();

        assert!(plan_config.has_pending_fields(), "fields should be pending");

        let res = plan_config.validate_templating_will_work();
        assert!(
            res.is_err(),
            "expected templating not to work, got: {res:?}"
        );

        let errs = res.unwrap_err().into_vec();
        let str_errs: Vec<String> = errs.iter().map(|e| format!("{:?}", e.kind)).collect();

        assert_eq!(str_errs.join("\n"), expected.trim());
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/valid-scenario-config-source")]
    #[tokio::test]
    async fn valid_scenario_config_source(_path: &str, content: &str) {
        let arr = load_archive(content);
        let content = get_file(&arr, "inline-content").to_string();
        let overrides: serde_yaml::Value =
            serde_yaml::from_str(get_file(&arr, "overrides")).unwrap();
        let expected: ScenarioConfig =
            serde_yaml::from_str(get_file(&arr, "expected-config")).unwrap();

        println!("{overrides:?}");

        let config_source: ConfigSource<ScenarioConfig> = ConfigSource::From {
            from: FileProvider::Inline(InlineFile { content }),
            overrides,
        };

        let ctx = Context::new(
            PathBuf::from("resources/config-tests/test-plan/valid-scenario-config-source")
                .canonicalize()
                .unwrap(),
        );

        let res = config_source.try_into_config(&ctx).await;
        assert!(res.is_ok(), "Expected a valid ScenarioConfig, got {res:?}");
        assert_eq!(res.unwrap(), expected, "expected scenario configs to match");
    }

    #[dir_cases(
        "crates/rtf-config/resources/config-tests/test-plan/valid-environment-config-source"
    )]
    #[tokio::test]
    async fn valid_environment_config_source(_path: &str, content: &str) {
        let arr = load_archive(content);
        let content = get_file(&arr, "inline-content").to_string();
        let overrides: serde_yaml::Value =
            serde_yaml::from_str(get_file(&arr, "overrides")).unwrap();
        let expected: EnvironmentConfig =
            serde_yaml::from_str(get_file(&arr, "expected-config")).unwrap();

        println!("{overrides:?}");

        let config_source: ConfigSource<EnvironmentConfig> = ConfigSource::From {
            from: FileProvider::Inline(InlineFile { content }),
            overrides,
        };

        let ctx = Context::new(
            PathBuf::from("resources/config-tests/test-plan/valid-environment-config-source")
                .canonicalize()
                .unwrap(),
        );

        let res = config_source.try_into_config(&ctx).await;
        assert!(
            res.is_ok(),
            "Expected a valid EnvironmentConfig, got {res:?}"
        );
        assert_eq!(
            res.unwrap(),
            expected,
            "expected environment configs to match"
        );
    }
}
