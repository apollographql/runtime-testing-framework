use crate::{
    checks::{self, Check},
    context::ResolutionContext,
    formats::{EnvironmentConfig, Error, Result, ScenarioConfig},
    providers::{
        self,
        command::CommandSection,
        file::{RawSource, Source},
    },
    templating::{self, Scalar, Template},
};
use itertools::Itertools;
use rtf_core::github::{self, Client};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    mem,
    path::Path,
};

/// The format for parsing scenario config
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
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

    /// Iteratate over all variants of this test plan that arise from
    /// [expanding](TestPlanConfig::expanded_matrix_values) any matrix values that it contains.
    ///
    /// This will always return at least the base test plan itself if there are no matrix values
    /// defined.
    pub fn iter_matrix_variants(&self) -> impl Iterator<Item = Self> {
        // TODO: RR-136 - replaces this with a custom iterator implementation that handles caching
        // providers that are shared between test plan variants.
        self.expanded_matrix_values().into_iter().map(|values| {
            let mut new = self.clone();
            new.values = values;
            new.matrix.clear();

            new
        })
    }

    pub fn n_matrix_variants(&self) -> usize {
        self.matrix
            .iter()
            .map(|(k, vals)| vals.iter().map(|v| (k.clone(), v.clone())))
            .multi_cartesian_product()
            .count()
    }

    /// The set of allowed templating values that this test plan supports.
    ///
    /// This is the union of values defined as a scalars and those that are part of a matrix
    fn allowed_values(&self) -> HashSet<&String> {
        self.values.keys().chain(self.matrix.keys()).collect()
    }

    /// We expand out matrix values as a cartesean product over all possible sets of values we can
    /// obtain when combined with any scalar values we have.
    pub fn expanded_matrix_values(&self) -> Vec<HashMap<String, Scalar>> {
        if self.matrix.is_empty() {
            return vec![self.values.clone()];
        }

        // Ensure that we have a consistent ordering for the vec we return.
        // The choice of ordering by the map key here is arbitrary but it is easy to document and
        // quickly check by hand for users when needed.
        let mut pairs: Vec<_> = self.matrix.iter().collect();
        pairs.sort_unstable_by(|(k1, _), (k2, _)| k1.cmp(k2));

        pairs
            .into_iter()
            .map(|(k, vals)| vals.iter().map(|v| (k.clone(), v.clone())))
            .multi_cartesian_product()
            .map(|matrix_vals| {
                let mut values = self.values.clone();
                values.extend(matrix_vals);
                values
            })
            .collect()
    }

    pub fn check_templating_will_work(&mut self) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();

        self.check_conflicting_keys(&mut errs);
        self.check_matrix_values(&mut errs);
        self.check_required_values(&mut errs);

        errs.into_result(())
    }

    /// Check if we have any conflicts between matrix values and scalar values
    fn check_conflicting_keys(&self, errs: &mut templating::ErrorBuilder) {
        let mut conflicting_keys: Vec<String> = self
            .values
            .keys()
            .filter(|k| self.matrix.contains_key(*k))
            .cloned()
            .collect();

        if !conflicting_keys.is_empty() {
            conflicting_keys.sort_unstable(); // ensure consistent ordering
            errs.push(
                templating::ErrorKind::ConflictingValues,
                conflicting_keys.join(", "),
                &[],
            )
        }
    }

    /// Check that all matrix arrays are non-empty and homogeneous
    fn check_matrix_values(&self, errs: &mut templating::ErrorBuilder) {
        for (k, vals) in self.matrix.iter() {
            let discriminant = match vals.first() {
                Some(val) => mem::discriminant(val),
                None => {
                    errs.push(templating::ErrorKind::EmptyMatrixValue, k, &[]);
                    continue;
                }
            };

            if !vals.iter().all(|v| mem::discriminant(v) == discriminant) {
                errs.push(templating::ErrorKind::InconsistentMatrixValue, k, &[]);
            }
        }
    }

    /// Check that all required values have been defined somewhere within the test plan
    fn check_required_values(&self, errs: &mut templating::ErrorBuilder) {
        let mut check_missing_values =
            |section: &CommandSection, allowed: &HashSet<&String>, error_path: &[String]| {
                let mut missing_values: Vec<String> = section
                    .required_values()
                    .iter()
                    .filter(|s| !allowed.contains(*s))
                    .cloned()
                    .collect();

                if !missing_values.is_empty() {
                    missing_values.sort_unstable(); // ensure consistent ordering
                    errs.push(
                        templating::ErrorKind::MissingValues,
                        missing_values.join(", "),
                        error_path,
                    )
                }
            };

        let mut allowed_values = self.allowed_values();

        check_missing_values(
            &self.environment.setup.command,
            &allowed_values,
            &["environment".to_string(), "setup".to_string()],
        );

        // The scenario and teardown are permitted to use values coming from setup.provides in
        // addition to the values declared in the test plan itself
        allowed_values.extend(self.environment.setup.provides.iter().map(|val| &val.name));

        check_missing_values(
            &self.environment.teardown,
            &allowed_values,
            &["environment".to_string(), "teardown".to_string()],
        );

        check_missing_values(
            &self.scenario.command,
            &allowed_values,
            &["scenario".to_string()],
        );
    }

    pub fn try_template_envrionment_setup(
        &mut self,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut path = vec!["environment".to_string()];
        self.environment.try_template_setup(&mut path, values)
    }

    pub fn try_template_envrionment_teardown(
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
            "environent",
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
        tp_source: Source,
        ctx: &impl ResolutionContext,
    ) -> Result<TestPlanConfig> {
        let (mut environment, environment_source) = self
            .environment
            .try_into_config_with_source(&tp_source, ctx)
            .await?;
        let (mut scenario, scenario_source) = self
            .scenario
            .try_into_config_with_source(&tp_source, ctx)
            .await?;

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
            sources: Sources::new(tp_source, scenario_source, environment_source),
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
    for item in mem::take(v).into_iter() {
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
        tp_source: &Source,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<(T, Option<Source>)> {
        match self {
            Self::Inline { inline } => Ok((inline, None)),
            Self::From { from, overrides } => {
                let src = from.try_into_source(tp_source, ctx)?;
                let file_content = src.try_get_file_content(ctx).await?;
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
    use crate::context::Context;
    use crate::txtar_context::TxtarContext;
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
        let src = Source::local(dir.join("test-plan.yaml"));

        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let mut expected: TestPlanConfig =
            serde_yaml::from_str(get_file(&arr, "expected")).unwrap();
        expected.sources.test_plan = src.clone();

        let raw_plan: RawTestPlanConfig = match serde_yaml::from_str(config) {
            Ok(plan) => plan,
            Err(e) => panic!("expected a valid RawTestPlanConfig, got: {e}"),
        };

        let res = raw_plan.try_into_test_plan(src.clone(), &ctx).await;
        assert!(res.is_ok(), "expected a test plan config, got: {res:?}");

        let plan = res.unwrap();
        let res = plan.try_check(&mut Vec::new(), &src, &ctx);
        assert!(
            res.is_ok(),
            "expected successful test plan check, got {res:?}"
        );
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

        // For matrices in this test we just want to check that things are valid so we only make
        // use of the first element for each value
        let mut combined_values = provides_values.clone();
        combined_values.extend(plan_config.expanded_matrix_values().remove(0));

        assert!(plan_config.has_pending_fields(), "fields should be pending");

        let res = plan_config.try_template(&mut Vec::new(), &combined_values);
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
        let expected: TestPlanConfig = serde_yaml::from_str(raw_expected).unwrap();

        // For matrices in this test we just want to check that things are valid so we only make
        // use of the first element for each value
        let values: HashMap<String, Scalar> = plan_config.expanded_matrix_values().remove(0);

        let res = plan_config.check_templating_will_work();
        assert!(res.is_ok(), "templating should work: {res:?}");
        assert!(plan_config.has_pending_fields(), "fields should be pending");

        let res = plan_config.try_template_envrionment_setup(&values);
        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(
            !plan_config.environment.setup.command.has_pending_fields(),
            "fields should be resolved"
        );

        let mut combined_values = provides_values.clone();
        combined_values.extend(values);

        let res = plan_config.try_template_envrionment_teardown(&combined_values);
        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(
            !plan_config.environment.teardown.has_pending_fields(),
            "fields should be resolved"
        );

        let res = plan_config.try_template_scenario(&combined_values);
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

        let res = plan_config.check_templating_will_work();
        assert!(
            res.is_err(),
            "expected templating not to work, got: {res:?}"
        );

        let errs = res.unwrap_err().into_vec();
        let str_errs: Vec<String> = errs.iter().map(|e| format!("{:?}", e.kind)).collect();

        assert_eq!(str_errs.join("\n"), expected.trim());
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

        let ctx = TxtarContext::new(arr);
        let res = config_source
            .try_into_config_with_source(&Source::local(""), &ctx)
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

        let ctx = TxtarContext::new(arr);

        let res = config_source
            .try_into_config_with_source(&Source::local(""), &ctx)
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

    macro_rules! values_map {
        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();
            $( m.insert($k.to_string(), $crate::templating::Scalar::try_from($v).unwrap()); )+
            m
        }};
    }

    #[test]
    fn matrix_value_expansion_works_without_any_matrix_values() {
        let tp = TestPlanConfig {
            values: values_map!("foo" => 42, "bar" => "life"),
            ..Default::default()
        };

        let all_values = tp.expanded_matrix_values();
        let expected = vec![values_map!("foo" => 42, "bar" => "life")];

        assert_eq!(all_values, expected);
    }

    #[test]
    fn matrix_value_expansion_works() {
        let tp = TestPlanConfig {
            values: values_map!("foo" => 42, "bar" => "life"),
            matrix: [
                ("baz".into(), vec![true.into(), false.into()]),
                ("qux".into(), vec![1.into(), 2.into()]),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let all_values = tp.expanded_matrix_values();
        let expected = vec![
            values_map!("foo" => 42, "bar" => "life", "baz" => true, "qux" => 1),
            values_map!("foo" => 42, "bar" => "life", "baz" => true, "qux" => 2),
            values_map!("foo" => 42, "bar" => "life", "baz" => false, "qux" => 1),
            values_map!("foo" => 42, "bar" => "life", "baz" => false, "qux" => 2),
        ];

        assert_eq!(all_values, expected);
        assert_eq!(tp.n_matrix_variants(), all_values.len());
    }
}
