use crate::{
    VariableDefinition,
    checks::{self, Check, CheckArrayDuplicates},
    context::ResolutionContext,
    formats::{
        CustomProviderDeclaration, EnvironmentConfig, Error, Matrix, Result, ScenarioConfig,
    },
    merge_yaml,
    providers::{
        self,
        file::{RawSource, SourceDir},
    },
    templating::{self, CustomProviderDefinitions, Scalar, Template, TemplateContext},
};
use itertools::Itertools;
use rtf_integrations::github::{self, Client};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::error;

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

    /// The set of allowed templating variables that this test plan supports.
    ///
    /// This is the union of variables defined as a scalars and those that are part of a matrix
    fn allowed_variables(&self) -> HashSet<&String> {
        self.variables
            .keys()
            .chain(self.matrix.keys())
            .chain(self.environment.setup.provides.iter().map(|val| &val.name))
            .collect()
    }

    pub fn check_templating_will_work(
        &mut self,
        override_sources: &HashMap<String, SourceDir>,
    ) -> templating::Result<()> {
        let stub_variables = self
            .allowed_variables()
            .into_iter()
            .map(|var| (var.to_string(), Scalar::String(var.to_string())))
            .collect();

        let ctx = TemplateContext::new(
            stub_variables,
            self.sources.test_plan.clone(),
            HashMap::new(),
            self.sources.custom_providers(),
        );

        let mut errs = templating::ErrorBuilder::new();
        self.validate_all_variable_definitions(&mut errs);

        let effective_allowed = self.compute_effective_allowed_values(&mut errs);
        self.validate_values_against_allowed(override_sources, &effective_allowed, &mut errs);

        self.matrix
            .check_conflicting_keys(&self.variables, &mut errs);
        self.matrix.check_dimensions(&mut errs);
        errs.append(self.validate_context(
            &mut Vec::new(),
            &HashSet::new(), // overwritten in self.validate_context
            self.sources.test_plan(),
            &ctx,
        ));

        errs.into_result(())
    }

    /// Validate all the the variable definitions
    fn validate_all_variable_definitions(&self, errs: &mut templating::ErrorBuilder) {
        fn validate_variable_definitions<'a>(
            variable_definitions: impl Iterator<Item = &'a VariableDefinition>,
            path: &[String],
            errs: &mut templating::ErrorBuilder,
        ) {
            for vd in variable_definitions {
                let mut full_path = path.to_vec();
                full_path.push("variable_definitions".to_string());
                full_path.push(vd.name.to_string());
                vd.validate(&full_path, errs);
            }
        }

        validate_variable_definitions(
            self.environment.variable_definitions.iter(),
            &["environment".to_string()],
            errs,
        );
        validate_variable_definitions(
            self.scenario.variable_definitions.iter(),
            &["scenario".to_string()],
            errs,
        );

        let custom_providers = self.sources.custom_providers();
        for (name, (_, def)) in custom_providers.test_plan.iter() {
            validate_variable_definitions(
                def.variable_definitions.iter(),
                &["custom_providers".to_string(), name.to_string()],
                errs,
            );
        }
        for (name, (_, def)) in custom_providers.scenario.iter() {
            validate_variable_definitions(
                def.variable_definitions.iter(),
                &["custom_providers".to_string(), name.to_string()],
                errs,
            );
        }
        for (name, (_, def)) in custom_providers.environment.iter() {
            validate_variable_definitions(
                def.variable_definitions.iter(),
                &["custom_providers".to_string(), name.to_string()],
                errs,
            );
        }
    }

    /// Collect all variable definitions grouped by name, with their source paths for error messages
    fn collect_variable_definitions_by_name(
        &self,
    ) -> HashMap<String, Vec<(Vec<String>, VariableDefinition)>> {
        let custom_providers = self.sources.custom_providers();
        let mut result: HashMap<String, Vec<(Vec<String>, VariableDefinition)>> = HashMap::new();

        for vd in &self.environment.variable_definitions {
            result.entry(vd.name.clone()).or_default().push((
                vec!["environment".into(), "variable_definitions".into()],
                vd.clone(),
            ));
        }

        for vd in &self.scenario.variable_definitions {
            result.entry(vd.name.clone()).or_default().push((
                vec!["scenario".into(), "variable_definitions".into()],
                vd.clone(),
            ));
        }

        for (name, (_, def)) in custom_providers.test_plan.iter() {
            for vd in &def.variable_definitions {
                result.entry(vd.name.clone()).or_default().push((
                    vec![
                        "custom_providers".into(),
                        name.clone(),
                        "variable_definitions".into(),
                    ],
                    vd.clone(),
                ));
            }
        }
        for (name, (_, def)) in custom_providers.scenario.iter() {
            for vd in &def.variable_definitions {
                result.entry(vd.name.clone()).or_default().push((
                    vec![
                        "custom_providers".into(),
                        name.clone(),
                        "variable_definitions".into(),
                    ],
                    vd.clone(),
                ));
            }
        }
        for (name, (_, def)) in custom_providers.environment.iter() {
            for vd in &def.variable_definitions {
                result.entry(vd.name.clone()).or_default().push((
                    vec![
                        "custom_providers".into(),
                        name.clone(),
                        "variable_definitions".into(),
                    ],
                    vd.clone(),
                ));
            }
        }

        result
    }

    /// Compute the effective allowed values for each variable by intersecting all definitions.
    /// Returns an empty vec for a variable if it's unconstrained (no allowed_values defined).
    /// Adds errors to `errs` if definitions have incompatible (empty intersection) allowed values.
    fn compute_effective_allowed_values(
        &self,
        errs: &mut templating::ErrorBuilder,
    ) -> HashMap<String, Vec<Scalar>> {
        let definitions_by_name = self.collect_variable_definitions_by_name();

        let mut effective: HashMap<String, Vec<Scalar>> = HashMap::new();

        for (var_name, definitions) in definitions_by_name {
            // Collect all Some(allowed_values) from definitions, skipping empty arrays
            // (empty arrays are already reported as EmptyAllowedValues errors)
            let constrained: Vec<_> = definitions
                .iter()
                .filter_map(|(path, vd)| {
                    vd.allowed_values
                        .as_ref()
                        .filter(|av| !av.is_empty())
                        .map(|av| (path, av))
                })
                .collect();

            if constrained.is_empty() {
                // All definitions have allowed_values: None -> unconstrained
                continue;
            }

            // Start with first constrained set, intersect with rest
            let mut intersection: Vec<Scalar> = constrained[0].1.clone();

            for (_, allowed) in constrained.iter().skip(1) {
                intersection.retain(|v| allowed.contains(v));
            }

            if intersection.is_empty() {
                // Build error message showing conflicting definitions
                let locations: Vec<_> = constrained
                    .iter()
                    .map(|(path, av)| format!("  - {}: {:?}", path.join("."), av))
                    .collect();

                errs.push(
                    templating::ErrorKind::IncompatibleAllowedValues,
                    format!(
                        "variable '{}' has incompatible allowed_values (no common values):\n{}",
                        var_name,
                        locations.join("\n")
                    ),
                    std::slice::from_ref(&var_name),
                );
            } else {
                effective.insert(var_name.clone(), intersection);
            }
        }

        effective
    }

    /// Validate that all variable values are in their effective allowed values.
    fn validate_values_against_allowed(
        &self,
        override_sources: &HashMap<String, SourceDir>,
        effective_allowed: &HashMap<String, Vec<Scalar>>,
        errs: &mut templating::ErrorBuilder,
    ) {
        // Check self.variables (includes CLI variables merged in)
        for (var_name, value) in &self.variables {
            if let Some(allowed) = effective_allowed.get(var_name)
                && !allowed.contains(value)
            {
                let source = if override_sources.contains_key(var_name) {
                    "CLI variable"
                } else {
                    "test plan variable"
                };
                errs.push(
                    templating::ErrorKind::ValueNotAllowed,
                    format!(
                        "{} '{}' has value '{}' not in allowed: {:?}",
                        source, var_name, value, allowed
                    ),
                    &["variables".into(), var_name.clone()],
                );
            }
        }

        // Check matrix dimensions
        for (var_name, values) in &self.matrix.dimensions {
            if let Some(allowed) = effective_allowed.get(var_name) {
                for value in values {
                    if !allowed.contains(value) {
                        let source = if override_sources.contains_key(var_name) {
                            "CLI matrix dimension"
                        } else {
                            "matrix dimension"
                        };
                        errs.push(
                            templating::ErrorKind::ValueNotAllowed,
                            format!(
                                "{} '{}' has value '{}' not in allowed: {:?}",
                                source, var_name, value, allowed
                            ),
                            &["matrix".into(), "dimensions".into(), var_name.clone()],
                        );
                    }
                }
            }
        }

        // Check matrix include
        for (idx, include_map) in self.matrix.include.iter().enumerate() {
            for (var_name, value) in include_map {
                if let Some(allowed) = effective_allowed.get(var_name)
                    && !allowed.contains(value)
                {
                    errs.push(
                        templating::ErrorKind::ValueNotAllowed,
                        format!(
                            "matrix include[{}] variable '{}' has value '{}' not in allowed: {:?}",
                            idx, var_name, value, allowed
                        ),
                        &[
                            "matrix".into(),
                            "include".into(),
                            idx.to_string(),
                            var_name.clone(),
                        ],
                    );
                }
            }
        }
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
        strip_sources_for_relative_paths(&mut val);

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

/// In order to be able to resolve relative paths we need to track where we obtained each of the
/// config files associated with a [TestPlan][TestPlanConfig].
///
/// If the [ScenarioConfig] or [EnvironmentConfig] are specified inline then their source will
/// match that of the overall [TestPlanConfig], otherwise we store the source as defined in the
/// [RawTestPlanConfig].
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct Sources {
    test_plan: SourceDir,
    scenario: Option<SourceDir>,
    environment: Option<SourceDir>,
    #[serde(default, skip)]
    custom_providers: Arc<CustomProviderDefinitions>,
}

impl Sources {
    fn new(
        test_plan: SourceDir,
        scenario: Option<SourceDir>,
        environment: Option<SourceDir>,
    ) -> Self {
        Self {
            test_plan,
            scenario,
            environment,
            custom_providers: Default::default(),
        }
    }

    async fn try_load_custom_providers(
        &mut self,
        tp: &[CustomProviderDeclaration],
        scenario: &[CustomProviderDeclaration],
        environment: &[CustomProviderDeclaration],
        ctx: &impl ResolutionContext,
    ) -> Result<()> {
        let mut errs = Vec::new();
        let mut custom_providers = CustomProviderDefinitions::default();

        let format_errors = |section: &str, errors: Vec<(String, providers::Error)>| {
            format!(
                "{section}:\n{}",
                errors
                    .into_iter()
                    .map(|(provider_name, e)| format!(" - {provider_name}: {e}"))
                    .join("\n")
            )
        };

        for declaration in tp.iter() {
            match declaration.try_load_all(self.test_plan(), ctx).await {
                Ok(providers) => custom_providers.test_plan.extend(providers),
                Err(errors) => errs.push(format_errors("test plan", errors)),
            }
        }

        for declaration in scenario.iter() {
            match declaration.try_load_all(self.scenario(), ctx).await {
                Ok(providers) => custom_providers.scenario.extend(providers),
                Err(errors) => errs.push(format_errors("scenario", errors)),
            }
        }

        for declaration in environment.iter() {
            match declaration.try_load_all(self.environment(), ctx).await {
                Ok(providers) => custom_providers.environment.extend(providers),
                Err(errors) => errs.push(format_errors("environment", errors)),
            }
        }

        if !errs.is_empty() {
            return Err(Error::FailedCustomProviderDefinitions { errs });
        }

        self.custom_providers = Arc::new(custom_providers);

        Ok(())
    }

    pub fn test_plan(&self) -> &SourceDir {
        &self.test_plan
    }

    /// The [Source] of the [EnvironmentConfig] in this test plan.
    ///
    /// Defaults to the source of the test plan itself if the environment was specified inline.
    pub fn environment(&self) -> &SourceDir {
        match self.environment.as_ref() {
            Some(source) => source,
            None => &self.test_plan,
        }
    }

    /// The [Source] of the [ScenarioConfig] in this test plan.
    ///
    /// Defaults to the source of the test plan itself if the scenario was specified inline.
    pub fn scenario(&self) -> &SourceDir {
        match self.scenario.as_ref() {
            Some(source) => source,
            None => &self.test_plan,
        }
    }

    pub fn custom_providers(&self) -> Arc<CustomProviderDefinitions> {
        self.custom_providers.clone()
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
    /// Templating variables to apply to fields within the rest of the test plan
    #[serde(default, alias = "values")]
    pub variables: HashMap<String, Scalar>,
    /// Sets of templating variables to apply to fields within the rest of the test plan as a matrix
    #[serde(default)]
    pub matrix: RawMatrix,
    /// Custom provider declarations to load for this test plan
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderDeclaration>,
    /// The test scenario to execute
    pub scenario: ConfigSpec,
    /// The environment setup and teardown to run around the test scenario
    pub environment: ConfigSpec,
}

impl RawTestPlanConfig {
    async fn try_into_test_plan(
        self,
        tp_source: SourceDir,
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

        let mut sources = Sources::new(tp_source, scenario_source, environment_source);
        sources
            .try_load_custom_providers(
                &self.custom_providers,
                &scenario.custom_providers,
                &environment.custom_providers,
                ctx,
            )
            .await?;

        Ok(TestPlanConfig {
            name: self.name,
            description: self.description,
            variables: self.variables,
            matrix: self.matrix.into(),
            custom_providers: self.custom_providers,
            scenario,
            environment,
            sources,
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
                include: Vec::new(),
            },
        }
    }
}

/// # Config Spec
///
/// This struct allows the config files to be resolved either from
/// inline yaml in the test plan or from a [crate::providers::file::FileProvider].
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
    /// directly from inline content or a [crate::providers::file::FileProvider]
    async fn try_into_config_with_source<T>(
        self,
        tp_source: &SourceDir,
        ctx: &impl ResolutionContext,
    ) -> Result<(T, Option<SourceDir>)>
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
                let (src, file_name) = from.try_into_source_and_filename(tp_source, ctx)?;
                let file_content = src.try_get_file_content(file_name, ctx).await?;
                let mut t: T = serde_yaml::from_str(&file_content)?;
                t.ensure_no_duplicate_keys()?;

                if overrides != serde_yaml::Value::Null {
                    if let Some(m) = overrides.as_mapping()
                        && m.contains_key("custom_providers")
                    {
                        return Err(Error::InvalidCustomProviderOverride);
                    }

                    let mut base: serde_yaml::Value = serde_yaml::from_str(&file_content)?;
                    let yaml_src = serde_yaml::to_value(tp_source)?;
                    set_source_for_relative_paths(&mut overrides, &yaml_src);
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

/// Relative files and Custom Providers defined as part of overrides need to be resolved relative
/// to the test plan source location rather than the source location of the file they are being
/// merged into.
///
/// To support this we tag any RelativeFile or CustomProvider file providers we can find with the
/// source of the test plan before we merge _at the YAML level_. We do it this way to avoid having
/// to define Rust types for the overrides where every field is optional, but this does mean that
/// we have zero type safety around this.
///
/// !! If something strange is happening around relative paths defined in test plan overrides then
///    this is likely the best place to start looking!
fn set_source_for_relative_paths(val: &mut serde_yaml::Value, src: &serde_yaml::Value) {
    use serde_yaml::Value;

    match val {
        Value::Mapping(map) => {
            let kind = map.get("kind").and_then(|v| v.as_str());
            if matches!(kind, Some("relative_path" | "custom_provider")) {
                map.insert(Value::String("src".into()), src.clone());
                return;
            }

            for v in map.values_mut() {
                set_source_for_relative_paths(v, src);
            }
        }

        Value::Sequence(seq) => {
            for v in seq {
                set_source_for_relative_paths(v, src);
            }
        }

        _ => (),
    }
}

/// This is the inverse of `set_source_for_relative_paths`.
///
/// We use this to keep sources as an internal detail of Test Plans when serializing out the
/// resolved Test Plan at the end of test runs.
pub(super) fn strip_sources_for_relative_paths(val: &mut serde_yaml::Value) {
    use serde_yaml::Value;

    match val {
        Value::Mapping(map) => {
            let kind = map.get("kind").and_then(|v| v.as_str());
            if matches!(kind, Some("relative_path" | "custom_provider")) {
                map.remove("src");
                return;
            }

            for v in map.values_mut() {
                strip_sources_for_relative_paths(v);
            }
        }

        Value::Sequence(seq) => {
            for v in seq {
                strip_sources_for_relative_paths(v);
            }
        }

        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        VariableDefinition,
        context::Context,
        formats::{
            custom_provider::CustomProviderDefinition,
            environment::{
                SetupSection,
                test_helpers::{environment_with_fields, templatable_environment},
            },
            scenario::test_helpers::{scenario_with_fields, templatable_scenario},
            tests::{
                assert_check_errors, assert_template_errors, expected_error_details,
                named_file_provider_with_field, p, r, templatable_file_providers, template_context,
                variable_definitions,
            },
        },
        providers::{
            command::{
                CommandProvider, CommandSection, CommandSpec,
                test_helpers::{cmd_with_inline_file, cmd_with_required_file},
            },
            file::{FileProvider, InlineFile, NamedFileProvider},
        },
        templating::{CustomProviderDefinitions, ErrorBuilder, ErrorKind, Field},
    };
    use assert_fs::{
        TempDir,
        prelude::{FileWriteStr, PathChild, PathCreateDir},
    };
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::path::PathBuf;

    // Helper functions

    /// Create a VariableDefinition with a default variable
    fn variable_with_default(name: &str, val: &str) -> VariableDefinition {
        VariableDefinition {
            name: name.into(),
            description: String::default(),
            default: Some(val.into()),
            allowed_values: None,
        }
    }

    /// Create a VariableDefinition with allowed_values
    fn variable_with_allowed_values(
        name: &str,
        default: Option<&str>,
        allowed_values: Option<Vec<&str>>,
    ) -> VariableDefinition {
        VariableDefinition {
            name: name.into(),
            description: String::default(),
            default: default.map(|v| v.into()),
            allowed_values: allowed_values.map(|vals| vals.into_iter().map(|v| v.into()).collect()),
        }
    }

    /// Create a HashMap of variables from key-value pairs using try_from
    macro_rules! variables_map {
        ($($k:expr => $v:expr),+) => {{
            let mut m = ::std::collections::HashMap::new();
            $( m.insert($k.to_string(), $crate::templating::Scalar::try_from($v).unwrap()); )+
            m
        }};
    }

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

    /// Create a TestPlanConfig for template tests
    fn templatable_test_plan(
        variables: HashMap<String, Scalar>,
        dimensions: HashMap<String, Vec<Scalar>>,
        include: Vec<HashMap<String, Scalar>>,
        scenario_fields: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        custom_providers: &[CustomProviderDeclaration],
    ) -> TestPlanConfig {
        let mut env_variables = setup_fields.to_vec();
        env_variables.extend_from_slice(teardown_fields);

        TestPlanConfig {
            variables,
            matrix: Matrix {
                variant_names: None,
                dimensions,
                include,
            },
            custom_providers: custom_providers.to_vec(),
            scenario: templatable_scenario(scenario_fields, scenario_fields, &[]),
            environment: templatable_environment(
                &env_variables,
                setup_fields,
                teardown_fields,
                &[],
            ),
            ..TestPlanConfig::empty()
        }
    }

    /// Create a matrix with two variables per key provided
    fn dimensions_from_keys(keys: &[&str], no_entries: isize) -> HashMap<String, Vec<Scalar>> {
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

    // Helper to create a TestPlanConfig with custom provider definitions for testing
    fn test_plan_with_custom_provider_definitions(
        custom_provider_variable_definitions: Vec<VariableDefinition>,
    ) -> TestPlanConfig {
        let custom_provider_def = CustomProviderDefinition {
            name: "test_provider".into(),
            description: "A test provider".into(),
            variable_definitions: custom_provider_variable_definitions,
            command: CommandSection::empty(),
        };

        let mut custom_providers = CustomProviderDefinitions::default();
        custom_providers.test_plan.insert(
            "test_provider".into(),
            (SourceDir::default(), custom_provider_def),
        );

        let sources = Sources {
            test_plan: SourceDir::default(),
            scenario: None,
            environment: None,
            custom_providers: Arc::new(custom_providers),
        };

        TestPlanConfig {
            sources,
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

        let expected_sources = Sources {
            test_plan: SourceDir::Local {
                abs_path: "/".into(),
            },
            scenario: None,
            environment: None,
            custom_providers: Default::default(),
        };

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

        assert_eq!(sources.custom_providers.test_plan.len(), 1);
        assert!(
            sources
                .custom_providers
                .test_plan
                .contains_key("tp_provider")
        );

        assert_eq!(sources.custom_providers.scenario.len(), 1);
        assert!(
            sources
                .custom_providers
                .scenario
                .contains_key("sc_provider")
        );

        assert_eq!(sources.custom_providers.environment.len(), 1);
        assert!(
            sources
                .custom_providers
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

        let expected_sources = Sources {
            test_plan: SourceDir::Local {
                abs_path: config_file_dir(&tp_file),
            },
            scenario: Some(SourceDir::Local {
                abs_path: config_file_dir(&scenario_file),
            }),
            environment: Some(SourceDir::Local {
                abs_path: config_file_dir(&environment_file),
            }),
            custom_providers: Default::default(),
        };

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

    // Tests for try_templating_will_work and its dependent functions
    #[test_case(&["scenario", "setup", "teardown"], &[], &[]; "all in variables")]
    #[test_case(&[], &["scenario", "setup", "teardown"], &[]; "all in dimensions")]
    #[test_case(&[], &[], &["scenario", "setup", "teardown"]; "all in include")]
    #[test_case(&["scenario"], &["setup"], &["teardown"]; "one in each")]
    #[test]
    fn check_templating_will_work_success(
        variable_keys: &[&str],
        dimension_keys: &[&str],
        include_keys: &[&str],
    ) {
        let variables = template_context(variable_keys);
        let matrix = dimensions_from_keys(dimension_keys, 1);
        let include = vec![template_context(include_keys).variables().clone()];
        let mut test_plan = templatable_test_plan(
            variables.variables().clone(),
            matrix,
            include,
            &["scenario"],
            &["setup"],
            &["teardown"],
            &[],
        );

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test_case(&["foo"], &[], "foo"; "single conflicting key dimensions and variables")]
    #[test_case(&["foo", "bar", "baz"], &[], "bar, baz, foo"; "multiple conflicting keys dimensions and variables")]
    #[test_case(&[], &["foo"], "foo"; "single conflicting key include and variables")]
    #[test_case(&[], &["foo", "bar", "baz"], "bar, baz, foo"; "multiple conflicting keys include and variables")]
    #[test_case(&["a"], &["a"], "a"; "single conflicting key dimensions and include")]
    #[test_case(&["a", "b", "c"], &["a", "b", "c"], "a, b, c"; "multiple conflicting keys dimensions and include")]
    #[test]
    fn check_templating_will_work_conflicting_keys_errors(
        dimension_keys: &[&str],
        include_keys: &[&str],
        expected_err_message: &str,
    ) {
        let variables = template_context(&["foo", "bar", "baz"]).variables().clone();
        let dimensions = dimensions_from_keys(dimension_keys, 2);
        let include = vec![template_context(include_keys).variables().clone()];
        let mut test_plan =
            templatable_test_plan(variables, dimensions, include, &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::ConflictingVariables;

        let res = test_plan.check_templating_will_work(&HashMap::new());
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
    fn check_templating_will_work_empty_dimension_errors(
        dimension_keys: &[&str],
        expected_err_messages: &[&str],
    ) {
        let variables = template_context(&[]).variables().clone();
        let dimensions = dimensions_from_keys(dimension_keys, 0);
        let include = Vec::new();
        let mut test_plan =
            templatable_test_plan(variables, dimensions, include, &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::EmptyMatrixVariable;

        let res = test_plan.check_templating_will_work(&HashMap::new());
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
                .all(|e| matches!(e.kind, ErrorKind::EmptyMatrixVariable)),
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
    fn check_templating_will_work_inconsistent_dimension_variable_errors() {
        let variables = HashMap::new();
        let mut dimensions: HashMap<String, Vec<Scalar>> = HashMap::new();
        dimensions.insert("foo".into(), vec!["a".into(), 42.into()]);
        let mut test_plan =
            templatable_test_plan(variables, dimensions, vec![], &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::InconsistentMatrixVariable;
        let expected_err_message = "foo";

        let res = test_plan.check_templating_will_work(&HashMap::new());
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

    #[test_case(vec![variables_map!("foo" => "a"), variables_map!("bar" => "b")]; "key names")]
    #[test_case(vec![variables_map!("foo" => "a"), variables_map!("foo" => 42)]; "variable types")]
    #[test]
    fn check_templating_will_work_inconsistent_include_errors(
        include: Vec<HashMap<String, Scalar>>,
    ) {
        let variables = HashMap::new();
        let dimensions = HashMap::new();
        let mut test_plan =
            templatable_test_plan(variables, dimensions, include, &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::InconsistentMatrixInclude;
        let expected_err_message = "matrix include maps must share consistent keys and types";

        let res = test_plan.check_templating_will_work(&HashMap::new());
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
    fn check_required_variables_setup_provides_available_to_scenario_and_teardown() {
        let provides = vec![VariableDefinition {
            name: "provides".to_string(),
            description: "A variable provided by setup".to_string(),
            default: None,
            allowed_values: None,
        }];

        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: variable_definitions(&["provides"]),
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
                variable_definitions: variable_definitions(&["provides"]),
                command: CommandSection {
                    file_providers: vec![named_file_provider_with_field("foo", p("provides"))],
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test_case(&["scenario"], &[], &[], &["scenario"]; "scenario missing variables")]
    #[test_case(&[], &["setup"], &[], &["setup"]; "setup missing variables")]
    #[test_case(&[], &[], &["teardown"], &["teardown"]; "teardown missing variables")]
    #[test_case(&[], &["setup"], &["teardown"], &["setup", "teardown"]; "setup and teardown missing variables")]
    #[test_case(&["scenario"], &["setup"], &[], &["setup", "scenario"]; "scenario and setup missing variables")]
    #[test_case(&["scenario"], &[], &["teardown"], &["teardown", "scenario"]; "scenario and teardown missing variables")]
    #[test_case(&["scenario"], &["setup"], &["teardown"], &["setup", "teardown", "scenario"]; "scenario and setup and teardown missing variables")]
    #[test]
    fn check_templating_will_work_missing_variables_errors(
        scenario_fields: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        expected_err_messages: &[&str],
    ) {
        let variables = template_context(&["foo"]).variables().clone();
        let dimensions = HashMap::new();
        let include = Vec::new();
        let mut test_plan = templatable_test_plan(
            variables,
            dimensions,
            include,
            scenario_fields,
            setup_fields,
            teardown_fields,
            &[],
        );

        let res = test_plan.check_templating_will_work(&HashMap::new());
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
                .all(|e| matches!(e.kind, ErrorKind::MissingVariable)),
            "expected all errors to be {:?}, got {:?}",
            ErrorKind::MissingVariable,
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
        let variables = template_context(&["foo", "bar"]).variables().clone();
        let dimensions = dimensions_from_keys(&["foo"], 0);
        let mut test_plan =
            templatable_test_plan(variables, dimensions, vec![], &["scenario"], &[], &[], &[]);

        let mut expected_errs = ErrorBuilder::new();
        expected_errs.push(
            ErrorKind::ConflictingVariables,
            "foo",
            &["test_plan".to_string()],
        );
        expected_errs.push(
            ErrorKind::EmptyMatrixVariable,
            "foo",
            &["test_plan".to_string()],
        );
        expected_errs.push(
            ErrorKind::MissingVariable,
            "  - scenario: \"description\"",
            &["scenario.file_providers.SCENARIO.path".to_string()],
        );
        let expected_errs = expected_errs.into_result("").unwrap_err();

        let res = test_plan.check_templating_will_work(&HashMap::new());
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
    fn check_required_variables_variable_definition_defaults_count_as_required_variables() {
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![
                    variable_with_default("setup", "setup"),
                    variable_with_default("teardown", "teardown"),
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
                variable_definitions: vec![variable_with_default("scenario", "scenario")],
                command: CommandSection {
                    file_providers: vec![named_file_provider_with_field("scenario", p("scenario"))],
                    ..CommandSection::empty()
                },
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test]
    fn check_templating_will_work_valid_allowed_values() {
        // Test that valid allowed_values configuration passes validation
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "env_var",
                    Some("a"),
                    Some(vec!["a", "b", "c"]),
                )],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "scenario_var",
                    None,
                    Some(vec!["x", "y"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed with valid allowed_values, got {:?}",
            res
        );
    }

    #[test_case(
        vec![variable_with_allowed_values("foo", None, Some(vec![]))],
        vec![],
        vec!["environment.variable_definitions.foo"];
        "single empty allowed_values in environment"
    )]
    #[test_case(
        vec![],
        vec![variable_with_allowed_values("bar", None, Some(vec![]))],
        vec!["scenario.variable_definitions.bar"];
        "single empty allowed_values in scenario"
    )]
    #[test_case(
        vec![variable_with_allowed_values("foo", None, Some(vec![]))],
        vec![variable_with_allowed_values("bar", None, Some(vec![]))],
        vec!["environment.variable_definitions.foo", "scenario.variable_definitions.bar"];
        "empty allowed_values in both environment and scenario"
    )]
    #[test_case(
        vec![
            variable_with_allowed_values("foo", None, Some(vec![])),
            variable_with_allowed_values("bar", None, Some(vec![]))
        ],
        vec![],
        vec!["environment.variable_definitions.foo", "environment.variable_definitions.bar"];
        "multiple empty allowed_values in environment"
    )]
    #[test]
    fn check_templating_will_work_empty_allowed_values_errors(
        env_variable_defs: Vec<VariableDefinition>,
        scenario_variable_defs: Vec<VariableDefinition>,
        expected_err_paths: Vec<&str>,
    ) {
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: env_variable_defs,
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: scenario_variable_defs,
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail for empty allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_err_paths.len(),
            "expected {} errors, got {:?}",
            expected_err_paths.len(),
            errors
        );

        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::EmptyAllowedValues)),
            "expected all errors to be EmptyAllowedValues, got {:?}",
            errors
        );

        let err_paths: Vec<&str> = errors.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            err_paths, expected_err_paths,
            "test that error paths are as expected"
        );
    }

    #[test_case(
        vec![variable_with_allowed_values("foo", Some("c"), Some(vec!["a", "b"]))],
        vec![],
        vec![("environment.variable_definitions.foo", "variable 'foo' has default 'c' not in allowed values")];
        "single default not in allowed values in environment"
    )]
    #[test_case(
        vec![],
        vec![variable_with_allowed_values("bar", Some("z"), Some(vec!["x", "y"]))],
        vec![("scenario.variable_definitions.bar", "variable 'bar' has default 'z' not in allowed values")];
        "single default not in allowed values in scenario"
    )]
    #[test_case(
        vec![variable_with_allowed_values("foo", Some("c"), Some(vec!["a", "b"]))],
        vec![variable_with_allowed_values("bar", Some("z"), Some(vec!["x", "y"]))],
        vec![
            ("environment.variable_definitions.foo", "variable 'foo' has default 'c' not in allowed values"),
            ("scenario.variable_definitions.bar", "variable 'bar' has default 'z' not in allowed values")
        ];
        "default not in allowed values in both environment and scenario"
    )]
    #[test]
    fn check_templating_will_work_default_not_in_allowed_values_errors(
        env_variable_defs: Vec<VariableDefinition>,
        scenario_variable_defs: Vec<VariableDefinition>,
        expected_errs: Vec<(&str, &str)>,
    ) {
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: env_variable_defs,
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: scenario_variable_defs,
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail for default not in allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_errs.len(),
            "expected {} errors, got {:?}",
            expected_errs.len(),
            errors
        );

        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::DefaultNotInAllowedValues)),
            "expected all errors to be DefaultNotInAllowedValues, got {:?}",
            errors
        );

        for (error, (expected_path, expected_msg_prefix)) in errors.iter().zip(expected_errs.iter())
        {
            assert_eq!(
                error.path, *expected_path,
                "test that error path is as expected"
            );
            assert!(
                error.message.starts_with(expected_msg_prefix),
                "expected message to start with '{}', got '{}'",
                expected_msg_prefix,
                error.message
            );
        }
    }

    #[test]
    fn check_templating_will_work_combined_allowed_values_errors() {
        // Test that empty allowed_values and default not in allowed_values are both caught
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![
                    variable_with_allowed_values("empty", None, Some(vec![])),
                    variable_with_allowed_values(
                        "invalid_default",
                        Some("c"),
                        Some(vec!["a", "b"]),
                    ),
                ],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "valid",
                    Some("x"),
                    Some(vec!["x", "y"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            2,
            "expected 2 errors (empty and invalid_default), got {:?}",
            errors
        );

        let err_kinds: Vec<_> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            err_kinds.contains(&&ErrorKind::EmptyAllowedValues),
            "expected EmptyAllowedValues error, got {:?}",
            err_kinds
        );
        assert!(
            err_kinds.contains(&&ErrorKind::DefaultNotInAllowedValues),
            "expected DefaultNotInAllowedValues error, got {:?}",
            err_kinds
        );
    }

    #[test]
    fn check_templating_will_work_custom_provider_valid_allowed_values() {
        let mut test_plan =
            test_plan_with_custom_provider_definitions(vec![variable_with_allowed_values(
                "env_type",
                Some("dev"),
                Some(vec!["dev", "staging", "prod"]),
            )]);

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed with valid custom provider allowed_values, got {:?}",
            res
        );
    }

    #[test]
    fn check_templating_will_work_custom_provider_empty_allowed_values_errors() {
        // Use None for default to avoid also triggering DefaultNotInAllowedValues
        let mut test_plan =
            test_plan_with_custom_provider_definitions(vec![variable_with_allowed_values(
                "env_type",
                None,
                Some(vec![]),
            )]);

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail for custom provider with empty allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            1,
            "expected 1 error, got {:?}",
            errors
        );

        let error = errors.iter().next().unwrap();
        assert!(
            matches!(error.kind, ErrorKind::EmptyAllowedValues),
            "expected EmptyAllowedValues error, got {:?}",
            error.kind
        );
        assert_eq!(
            error.path, "custom_providers.test_provider.variable_definitions.env_type",
            "expected error path to reference custom provider"
        );
    }

    #[test]
    fn check_templating_will_work_custom_provider_default_not_in_allowed_values_errors() {
        let mut test_plan =
            test_plan_with_custom_provider_definitions(vec![variable_with_allowed_values(
                "env_type",
                Some("test"),
                Some(vec!["dev", "staging", "prod"]),
            )]);

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail for custom provider with default not in allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            1,
            "expected 1 error, got {:?}",
            errors
        );

        let error = errors.iter().next().unwrap();
        assert!(
            matches!(error.kind, ErrorKind::DefaultNotInAllowedValues),
            "expected DefaultNotInAllowedValues error, got {:?}",
            error.kind
        );
        assert_eq!(
            error.path, "custom_providers.test_provider.variable_definitions.env_type",
            "expected error path to reference custom provider"
        );
        assert!(
            error.message.contains("env_type"),
            "expected error message to contain variable name, got '{}'",
            error.message
        );
    }

    #[test]
    fn check_templating_will_work_compatible_allowed_values_across_configs() {
        // Scenario and environment both define 'foo' with overlapping allowed_values
        let mut test_plan = TestPlanConfig {
            variables: [("foo".into(), "a".into())].into(),
            environment: EnvironmentConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b", "c"]),
                )],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b", "d"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_ok(),
            "expected compatible allowed_values to succeed, got {:?}",
            res
        );
    }

    #[test]
    fn check_templating_will_work_incompatible_allowed_values_errors() {
        // Scenario and environment both define 'foo' with NO overlapping allowed_values
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b"]),
                )],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["c", "d"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(res.is_err(), "expected incompatible allowed_values to fail");

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.kind, ErrorKind::IncompatibleAllowedValues)),
            "expected IncompatibleAllowedValues error, got {:?}",
            errors
        );
        assert!(
            errors.iter().any(|e| e.message.contains("foo")),
            "expected error message to contain variable name 'foo'"
        );
    }

    #[test_case(
        Matrix {
            dimensions: [("foo".into(), vec!["x".into(), "y".into()])].into(),
            ..Default::default()
        },
        "dimensions";
        "matrix dimension value not allowed"
    )]
    #[test_case(
        Matrix {
            include: vec![[("foo".into(), "x".into())].into()],
            ..Default::default()
        },
        "include";
        "matrix include value not allowed"
    )]
    #[test]
    fn check_templating_will_work_matrix_value_not_allowed(
        matrix: Matrix,
        expected_path_part: &str,
    ) {
        let mut test_plan = TestPlanConfig {
            matrix,
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new());
        assert!(
            res.is_err(),
            "expected matrix {} with invalid value to fail",
            expected_path_part
        );

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.kind, ErrorKind::ValueNotAllowed)),
            "expected ValueNotAllowed error, got {:?}",
            errors
        );
        assert!(
            errors.iter().any(|e| e.path.contains(expected_path_part)),
            "expected error path to contain '{}', got {:?}",
            expected_path_part,
            errors
        );
    }

    #[test_case(
        false,
        "test plan variable";
        "test plan variable not allowed"
    )]
    #[test_case(
        true,
        "CLI variable";
        "CLI variable not allowed"
    )]
    #[test]
    fn check_templating_will_work_variable_not_allowed(
        is_cli_override: bool,
        expected_source: &str,
    ) {
        let mut test_plan = TestPlanConfig {
            variables: [("foo".into(), "x".into())].into(),
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let override_sources: HashMap<String, SourceDir> = if is_cli_override {
            [("foo".into(), SourceDir::local("/cli"))].into()
        } else {
            HashMap::new()
        };

        let res = test_plan.check_templating_will_work(&override_sources);
        assert!(
            res.is_err(),
            "expected {} with invalid value to fail",
            expected_source
        );

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.kind, ErrorKind::ValueNotAllowed)),
            "expected ValueNotAllowed error, got {:?}",
            errors
        );
        assert!(
            errors.iter().any(|e| e.message.contains(expected_source)),
            "expected error message to identify source as {}, got {:?}",
            expected_source,
            errors
        );
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
