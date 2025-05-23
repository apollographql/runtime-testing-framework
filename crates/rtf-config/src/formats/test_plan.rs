//! Parsing for the test plan and base test plan config file formats
use crate::{
    ValueDefinition,
    formats::{Error, Result},
    validation::{self, duplicate_keys},
};

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BaseTestPlanConfig {
    pub name: String,
    pub description: String,
    pub scenario_defines: Vec<ValueDefinition>,
    pub environment_provides: Vec<ValueDefinition>,
}

impl BaseTestPlanConfig {
    pub fn try_load_and_resolve(p: impl Into<PathBuf>) -> Result<Self> {
        let p = p.into();
        let raw = RawBaseTestPlanConfig::try_load_from_path(&p)?;

        raw.try_validate_and_resolve()
    }

    pub fn try_resolve_from_str(s: &str) -> Result<Self> {
        let raw = RawBaseTestPlanConfig::from_str(s)?;

        raw.try_validate_and_resolve()
    }
}

/// The raw serialization format for parsing user provided base test plan config.
#[derive(Debug, Clone, Deserialize)]
pub struct RawBaseTestPlanConfig {
    pub name: String,
    pub description: String,
    pub scenario_defines: Vec<ValueDefinition>,
    pub environment_provides: Vec<ValueDefinition>,
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

    /// [Validate][Self::validate] this config file before converting it to a [BaseTestPlanConfig].
    pub fn try_validate_and_resolve(self) -> Result<BaseTestPlanConfig> {
        self.validate()?;

        Ok(self.into_base_test_plan_config())
    }

    #[inline]
    fn into_base_test_plan_config(self) -> BaseTestPlanConfig {
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
    use rtf_test_utils;
    use simple_test_case::dir_cases;
    use simple_txtar::Archive;

    #[dir_cases("crates/rtf-config/resources/base-test-plan-config-tests")]
    #[test]
    fn base_test_plan_config_scenarios(_path: &str, content: &str) {
        let arr = Archive::from(content);

        let comment = arr.comment();
        if !comment.is_empty() {
            println!("{}", comment.trim());
        }

        let config = match arr.get("config.yaml") {
            Some(f) => f.content.trim(),
            None => {
                panic!("Error: 'config.yaml' not found in the archive");
            }
        };

        // TO DO: For negative test scenarios we need to check whether one of expected-file-content
        // or the expected-errors object exists. If neither exists we need to panic.
        let expected_json = arr.get("expected-json");

        let res: serde_yaml::Result<RawBaseTestPlanConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let raw = res.unwrap();

        let res = raw.validate();
        assert!(res.is_ok(), "failed to validate: {res:?}");

        let resolved_config = raw.into_base_test_plan_config();
        let res = rtf_test_utils::to_pretty_json_with_indent(&resolved_config, 4);

        if let Some(expected) = expected_json {
            assert_eq!(res, expected.content.trim());
        }
    }
}
