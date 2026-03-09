use crate::{
    checks::CheckArrayDuplicates,
    context::ResolutionContext,
    formats::{
        CustomProviderDeclaration, EnvironmentConfig, Error, Matrix, Result, ScenarioConfig,
        TestPlanConfig, test_plan::Sources,
    },
    merge_yaml,
    providers::file::{RawSource, SourceDir, StableSource},
    templating::Scalar,
};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use tracing::error;

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
    pub(super) async fn try_into_test_plan(
        self,
        tp_source: SourceDir,
        ctx: &impl ResolutionContext,
    ) -> Result<(TestPlanConfig, Sources)> {
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

        Ok((
            TestPlanConfig {
                name: self.name,
                description: self.description,
                variables: self.variables,
                matrix: self.matrix.into(),
                custom_providers: self.custom_providers,
                scenario,
                environment,
            },
            sources,
        ))
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
                    let yaml_src = serde_yaml::to_value(&StableSource::TestPlan)?;
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
            if matches!(
                kind,
                Some("relative_path" | "relative_dir" | "custom_provider")
            ) {
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
