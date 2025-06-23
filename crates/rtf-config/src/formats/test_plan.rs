use crate::{
    context::ResolutionContext,
    formats::{EnvironmentConfig, Error, Result, ScenarioConfig},
    providers::{
        self,
        file::{RawSource, Source},
    },
    templating::{self, Scalar, Template},
    validation::{self, Validate},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::HashMap, hash::Hash, mem::take, path::Path};

/// The format for parsing scenario config
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TestPlanConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub values: HashMap<String, Scalar>,
    #[serde(default)]
    pub matrix: HashMap<String, Vec<Scalar>>,
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

        raw.try_into_test_plan(&abs_path, ctx).await
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
        let mut path = vec!["environment".to_string()];
        let res = self.environment.try_resolve_setup(&mut path, values);

        debug_assert!(
            !self.environment.setup.command.has_pending_fields(),
            "Envrionment setup should not have pending fields"
        );

        res
    }

    pub fn try_resolve_envrionment_teardown(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut path = vec!["environment".to_string()];
        let res = self.environment.try_resolve_teardown(&mut path, values);

        debug_assert!(
            !self.environment.teardown.has_pending_fields(),
            "Envrionment teardown should not have pending fields"
        );

        res
    }

    pub fn try_resolve_scenario(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut path = vec!["scenario".to_string()];
        let res = self.scenario.try_resolve(&mut path, values);

        debug_assert!(
            !self.scenario.has_pending_fields(),
            "Scenario should not have pending fields"
        );

        res
    }

    pub async fn run_environment_setup(
        &self,
        out_dir: &Path,
        ctx: &impl ResolutionContext,
    ) -> Result<HashMap<String, Scalar>> {
        let raw_output = self
            .environment
            .setup
            .command
            .run_providers_and_execute(out_dir, self.sources.environment(), ctx)
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
        ctx: &impl ResolutionContext,
    ) -> Result<()> {
        self.environment
            .teardown
            .run_providers_and_execute(out_dir, self.sources.environment(), ctx)
            .await?;

        Ok(())
    }

    pub async fn run_scenario(&self, out_dir: &Path, ctx: &impl ResolutionContext) -> Result<()> {
        self.scenario
            .command
            .run_providers_and_execute(out_dir, self.sources.scenario(), ctx)
            .await?;

        Ok(())
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
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.environment.try_resolve_nested(
            path,
            "environment",
            values,
        ));
        errs.append(self.scenario.try_resolve_nested(path, "scenario", values));

        errs.into_result(())
    }
}

impl Validate for TestPlanConfig {
    fn try_validate(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::from(self.environment.try_validate_nested(
            path,
            "environent",
            src,
            ctx,
        ));
        errs.append(
            self.scenario
                .try_validate_nested(path, "scenario", src, ctx),
        );

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
    fn new(abs_path: &Path, scenario: Option<Source>, environment: Option<Source>) -> Self {
        Self {
            test_plan: Source::local(abs_path),
            scenario,
            environment,
        }
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
/// being fully templated and validated.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RawTestPlanConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub values: HashMap<String, Scalar>,
    #[serde(default)]
    pub matrix: HashMap<String, Vec<Scalar>>,
    pub scenario: ConfigSpec<ScenarioConfig>,
    pub environment: ConfigSpec<EnvironmentConfig>,
}

impl RawTestPlanConfig {
    async fn try_into_test_plan(
        self,
        abs_path: &Path,
        ctx: &impl ResolutionContext,
    ) -> Result<TestPlanConfig> {
        let dir = abs_path.parent().expect("we know we have a parent");
        let (mut environment, environment_source) = self
            .environment
            .try_into_config_with_source(dir, ctx)
            .await?;
        let (mut scenario, scenario_source) =
            self.scenario.try_into_config_with_source(dir, ctx).await?;

        dedup_and_sort_by_key(&mut environment.values, |v| v.name.clone());
        dedup_and_sort_by_key(&mut environment.setup.provides, |v| v.name.clone());
        dedup_and_sort_by_key(&mut environment.setup.command.file_providers, |nfp| {
            nfp.env_var.clone()
        });
        dedup_and_sort_by_key(&mut environment.teardown.file_providers, |nfp| {
            nfp.env_var.clone()
        });
        dedup_and_sort_by_key(&mut scenario.values, |v| v.name.clone());
        dedup_and_sort_by_key(&mut scenario.command.file_providers, |nfp| {
            nfp.env_var.clone()
        });

        Ok(TestPlanConfig {
            name: self.name,
            description: self.description,
            values: self.values,
            matrix: self.matrix,
            scenario,
            environment,
            sources: Sources::new(abs_path, scenario_source, environment_source),
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
pub enum ConfigSpec<T> {
    Inline {
        inline: T,
    },
    From {
        from: RawSource,
        #[serde(default)]
        overrides: serde_yaml::Value,
    },
}

impl<T> ConfigSpec<T>
where
    T: DeserializeOwned,
{
    /// Try to convert the [ConfigSpec] into a config yaml. This can read the content
    /// directly from inline content or a [FileProvider]
    async fn try_into_config_with_source(
        self,
        dir: &Path,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<(T, Option<Source>)> {
        match self {
            Self::Inline { inline } => Ok((inline, None)),
            Self::From { from, overrides } => {
                let file_content = from.try_get_file_content(dir, ctx).await?;
                let src = from.try_into_source(dir, ctx)?;
                let mut base: serde_yaml::Value = serde_yaml::from_str(&file_content)?;
                if overrides != serde_yaml::Value::Null {
                    merge(overrides, &mut base);
                }

                Ok((serde_yaml::from_value(base)?, Some(src)))
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
    use crate::context::{Context, NullPlatformClient, PathKind, ResolutionContext};
    use simple_test_case::dir_cases;
    use simple_txtar::Archive;
    use std::{io, path::PathBuf};

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
            Some(f) => &f.content,
            None => {
                panic!("required txtar file section {fname:?} was missing");
            }
        }
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/valid")]
    #[tokio::test]
    async fn valid_config(_path: &str, content: &str) {
        let dir = PathBuf::from("resources/config-tests/test-plan/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(&dir);

        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let mut expected: TestPlanConfig =
            serde_yaml::from_str(get_file(&arr, "expected")).unwrap();
        expected.sources.test_plan = src.clone();

        let raw_plan: RawTestPlanConfig = match serde_yaml::from_str(config) {
            Ok(plan) => plan,
            Err(e) => panic!("expected a valid RawTestPlanConfig, got: {e}"),
        };

        let res = raw_plan.try_into_test_plan(&dir, &ctx).await;
        assert!(res.is_ok(), "expected a test plan config, got: {res:?}");

        let plan = res.unwrap();
        let res = plan.try_validate(&mut Vec::new(), &src, &ctx);
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

    struct TxtarContext {
        arr: Archive,
    }

    impl ResolutionContext for TxtarContext {
        type PlatformClient = NullPlatformClient;

        fn run_command_blocking<'a>(
            &self,
            _prog: &str,
            _args: impl IntoIterator<Item = &'a str>,
            _env_vars: &HashMap<String, String>,
        ) -> io::Result<()> {
            Ok(())
        }

        fn write(&self, _path: impl AsRef<Path>, _content: impl AsRef<[u8]>) -> io::Result<()> {
            unimplemented!()
        }

        fn path_kind(&self, _path: impl AsRef<Path>) -> crate::context::PathKind {
            PathKind::File
        }

        fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
            Ok(relative_path.as_ref().to_path_buf())
        }

        fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
            let p = path.as_ref().display().to_string();

            self.arr
                .get(&p)
                .map(|f| f.content.clone())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, ""))
        }

        fn remove_file(&self, _path: impl AsRef<Path>) -> io::Result<()> {
            Ok(())
        }

        fn set_current_dir(&mut self, _path: impl AsRef<Path>) -> io::Result<()> {
            Ok(())
        }

        fn create_dir_all(&self, _path: impl AsRef<Path>) -> io::Result<()> {
            Ok(())
        }
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/valid-scenario-overrides")]
    #[tokio::test]
    async fn valid_scenario_overrides(_path: &str, content: &str) {
        let arr = load_archive(content);
        let overrides: serde_yaml::Value =
            serde_yaml::from_str(get_file(&arr, "overrides")).unwrap();
        let expected: ScenarioConfig =
            serde_yaml::from_str(get_file(&arr, "expected-config")).unwrap();

        let config_source: ConfigSpec<ScenarioConfig> = ConfigSpec::From {
            from: RawSource::Local {
                relative_path: PathBuf::from("scenario.yaml"),
            },
            overrides,
        };

        let ctx = TxtarContext { arr };
        let res = config_source
            .try_into_config_with_source(&PathBuf::new(), &ctx)
            .await;
        assert!(res.is_ok(), "Expected a valid ScenarioConfig, got {res:?}");
        assert_eq!(
            res.unwrap().0,
            expected,
            "expected scenario configs to match"
        );
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/test-plan/valid-environment-overrides")]
    #[tokio::test]
    async fn valid_environment_overrides(_path: &str, content: &str) {
        let arr = load_archive(content);
        let overrides: serde_yaml::Value =
            serde_yaml::from_str(get_file(&arr, "overrides")).unwrap();
        let expected: EnvironmentConfig =
            serde_yaml::from_str(get_file(&arr, "expected-config")).unwrap();

        let config_source: ConfigSpec<EnvironmentConfig> = ConfigSpec::From {
            from: RawSource::Local {
                relative_path: PathBuf::from("environment.yaml"),
            },
            overrides,
        };

        let ctx = TxtarContext { arr };

        let res = config_source
            .try_into_config_with_source(&PathBuf::new(), &ctx)
            .await;
        assert!(
            res.is_ok(),
            "Expected a valid EnvironmentConfig, got {res:?}"
        );
        assert_eq!(
            res.unwrap().0,
            expected,
            "expected environment configs to match"
        );
    }
}
