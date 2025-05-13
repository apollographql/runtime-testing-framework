//! Parsing for the test plan and base test plan config file formats
use crate::{
    ValueSchema,
    formats::{Error, Result},
    validation::{self, duplicate_keys},
};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, Clone, PartialEq)]
pub struct BaseTestPlanConfig {
    pub name: String,
    pub description: String,
    pub scenario_defines: Vec<ValueSchema>,
    pub environment_provides: Vec<ValueSchema>,
}

impl BaseTestPlanConfig {
    pub fn try_load_and_resolve(p: impl Into<PathBuf>) -> Result<Self> {
        let p = p.into();
        let raw = RawBaseTestPlanConfig::try_load_from_path(&p)?;
        raw.validate()?;

        Ok(raw.into_resolved_unchecked())
    }

    pub fn try_resolve_from_str(s: &str) -> Result<Self> {
        let raw = RawBaseTestPlanConfig::from_str(s)?;
        raw.validate()?;

        Ok(raw.into_resolved_unchecked())
    }
}

/// The raw serialization format for parsing user provided base test plan config.
#[derive(Debug, Clone, Deserialize)]
pub struct RawBaseTestPlanConfig {
    pub name: String,
    pub description: String,
    pub scenario_defines: Vec<ValueSchema>,
    pub environment_provides: Vec<ValueSchema>,
}

impl FromStr for RawBaseTestPlanConfig {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let raw: Self = serde_yaml::from_str(s)?;

        Ok(raw)
    }
}

impl RawBaseTestPlanConfig {
    /// Attempt to load and parse a YAML config file as [RawBaseTestPlanConfig].
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Self::from_str(&content)
    }

    /// Check to see if there were any duplicated value names within either of the scenario_defines
    /// or environment_provides sections of the config file. We allow for the use of the same key
    /// name between the two sections as they are namespaced when they are made available within
    /// other files as template variables.
    pub fn validate(&self) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        let scenario_duplicates = duplicate_keys(self.scenario_defines.iter(), |s| &s.name);
        if !scenario_duplicates.is_empty() {
            errs.push(
                validation::ErrorKind::DuplicateValueNames,
                format!("scenario_defines:\n  {}", scenario_duplicates.join("\n  ")),
            );
        }

        let env_duplicates = duplicate_keys(self.environment_provides.iter(), |s| &s.name);
        if !env_duplicates.is_empty() {
            errs.push(
                validation::ErrorKind::DuplicateValueNames,
                format!("environment_provides:\n  {}", env_duplicates.join("\n  ")),
            );
        }

        // TODO: all value schemas need to be checked to see if they are actually valid JSON-schema
        // schemas

        errs.into_result(())
    }

    /// No programmatic transformation is needed in order to convert a [RawBaseTestPlanConfig] into
    /// a [BaseTestPlanConfig] but calling this method may result in a degraded debugging
    /// experience for users if this raw config has not already been validated.
    ///
    /// You should always prefer calling [BaseTestPlanConfig::try_load_and_resolve] or
    /// [BaseTestPlanConfig::try_resolve_from_str] where possible.
    pub fn into_resolved_unchecked(self) -> BaseTestPlanConfig {
        BaseTestPlanConfig {
            name: self.name,
            description: self.description,
            scenario_defines: self.scenario_defines,
            environment_provides: self.environment_provides,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use simple_test_case::dir_cases;

    #[dir_cases("crates/rtf-config/resources/base-test-plan-config-tests/valid")]
    #[tokio::test]
    async fn valid_environment_config_parses_and_resolves(_path: &str, content: &str) {
        let raw: RawBaseTestPlanConfig =
            serde_yaml::from_str(content).expect("to parse with serde");
        let res = raw.validate();
        assert!(res.is_ok(), "failed to validate: {res:?}");
    }

    #[tokio::test]
    async fn minimal_base_test_plan_config_resolves_correctly() {
        let content =
            include_str!("../../resources/base-test-plan-config-tests/valid/minimal.yaml");
        let res = BaseTestPlanConfig::try_resolve_from_str(content);
        assert!(res.is_ok(), "failed to resolve config file: {res:?}");

        let cfg = res.unwrap();

        let expected = BaseTestPlanConfig {
            name: "minimal".to_string(),
            description: "a minimal test plan base".to_string(),
            scenario_defines: vec![ValueSchema {
                name: "supergraph_schema".to_string(),
                description: "The supergraph that should be run by the Router".to_string(),
                schema: Some(json!({
                    "type": "object",
                    "properties": json!({
                        "file": json!({
                            "$ref": "#/definitions/rtf-supergraph-schema"
                        })
                    })
                })),
            }],
            environment_provides: vec![ValueSchema {
                name: "subgraph_urls".to_string(),
                description: "A map of subgraph names to their override URL".to_string(),
                schema: Some(json!({
                    "type": "object",
                    "additionalProperties": json!({
                        "type": "string"
                    })
                })),
            }],
        };

        assert_eq!(cfg, expected);
    }
}
