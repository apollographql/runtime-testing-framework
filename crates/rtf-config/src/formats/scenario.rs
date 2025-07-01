use crate::{
    ValueDefinition,
    checks::{self, Check},
    context::ResolutionContext,
    formats::{Result, filter_values},
    providers::{command::CommandSection, file::Source},
    templating::{self, Scalar, Template},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

/// The format for parsing scenario config
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct ScenarioConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub values: Vec<ValueDefinition>,
    #[serde(flatten)]
    pub command: CommandSection,
}

impl ScenarioConfig {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }
}

impl Template for ScenarioConfig {
    fn has_pending_fields(&self) -> bool {
        self.command.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        self.command.required_values()
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let definitions = self.values.iter();
        let allowed_values = filter_values(values, definitions);

        self.command
            .try_template_nested(path, "command_section", &allowed_values)
    }
}

impl Check for ScenarioConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        self.command.try_check_nested(path, "command", src, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::Context, templating::Scalar};
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

    #[dir_cases("crates/rtf-config/resources/config-tests/scenario/valid")]
    #[tokio::test]
    async fn valid_config(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let scenario: ScenarioConfig = match serde_yaml::from_str(config) {
            Ok(scenario) => scenario,
            Err(e) => panic!("expected a valid ScenarioConfig, got: {e}"),
        };

        let dir = PathBuf::from("resources/config-tests/scenario/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir);

        let res = scenario.try_check(&mut Vec::new(), &src, &ctx);
        assert!(res.is_ok(), "expected successful check but got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/scenario/check-failures")]
    #[test]
    fn check_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "check-errors");

        let res: serde_yaml::Result<ScenarioConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let dir = PathBuf::from("resources/config-tests/scenario/check-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir);

        let scenario_config = res.unwrap();
        let res = scenario_config.try_check(&mut Vec::new(), &src, &ctx);

        assert!(res.is_err(), "expected check failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
        );
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/scenario/parse-failures")]
    #[test]
    fn parse_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let res: serde_yaml::Result<ScenarioConfig> = serde_yaml::from_str(config);

        assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/scenario/valid-templates")]
    #[test]
    fn valid_templates(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let raw_expected = get_file(&arr, "after-templating");

        let mut scenario_config: ScenarioConfig = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();
        let expected: ScenarioConfig = serde_yaml::from_str(raw_expected).unwrap();

        assert!(
            scenario_config.has_pending_fields(),
            "fields should be pending"
        );

        let res = scenario_config.try_template(&mut Vec::new(), &values);

        assert!(res.is_ok(), "expected no errors, got {res:?}");
        assert!(
            !scenario_config.has_pending_fields(),
            "fields should be resolved"
        );
        assert_eq!(scenario_config, expected);
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/scenario/invalid-templates")]
    #[test]
    fn invalid_templates(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let expected = get_file(&arr, "templating-errors");

        let mut scenario_config: ScenarioConfig = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();

        assert!(
            scenario_config.has_pending_fields(),
            "fields should be pending"
        );

        let res = scenario_config.try_template(&mut Vec::new(), &values);

        assert!(
            scenario_config.has_pending_fields(),
            "fields should still be pending"
        );

        let errs = res.unwrap_err().into_vec();
        let str_errs: Vec<String> = errs.iter().map(|e| format!("{:?}", e.kind)).collect();

        assert_eq!(str_errs.join("\n"), expected.trim());
    }
}
