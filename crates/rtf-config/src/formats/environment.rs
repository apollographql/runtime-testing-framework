//! Parsing of the environment provisioner config file format
use crate::{
    ValueDefinition,
    formats::Result,
    providers::{Context, command::CommandSection},
    templating::{self, Scalar, Templatable},
    validation::{self, duplicate_keys},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EnvironmentConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub values: Vec<ValueDefinition>,
    pub setup: SetupSection,
    pub teardown: CommandSection,
}

impl EnvironmentConfig {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    pub fn validate(&self, ctx: &Context) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        // Check that hard coded values and the ones coming from setup.provides are unique
        let all_values = self.values.iter().chain(self.setup.provides.iter());
        let duplicates = duplicate_keys(all_values, |v| &v.name);
        if !duplicates.is_empty() {
            errs.push(
                validation::ErrorKind::DuplicateValueNames,
                duplicates.join("\n"),
            );
        }

        // Check that each command is valid in isolation
        if let Err(e) = self.setup.command.validate(ctx) {
            errs.extend_with_prefix(e, "setup");
        }

        if let Err(e) = self.teardown.validate(ctx) {
            errs.extend_with_prefix(e, "teardown");
        }

        errs.into_result(())
    }

    /// Try to resolve the setup [CommandSection].
    ///
    /// Setup is only allowed to reference values that are declared in the values section of this
    /// config file.
    pub fn try_resolve_setup(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
        errs: &mut Vec<templating::Error>,
    ) {
        let allowed_values: HashMap<String, Scalar> = values
            .iter()
            .filter(|(k, _)| self.values.iter().any(|val| &val.name == *k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        self.setup
            .command
            .try_resolve_nested(path, "setup", &allowed_values, errs);
    }

    /// Try to resolve the teardown [CommandSection].
    ///
    /// Teardown is allowed to reference values that come from the output of setup in addition to
    /// the values decalered in the values section of this config file.
    pub fn try_resolve_teardown(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
        errs: &mut Vec<templating::Error>,
    ) {
        let allowed_values: HashMap<String, Scalar> = values
            .iter()
            .filter(|(k, _)| {
                self.values.iter().any(|val| &val.name == *k)
                    || self.setup.provides.iter().any(|val| &val.name == *k)
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        self.teardown
            .try_resolve_nested(path, "teardown", &allowed_values, errs);
    }
}

impl Templatable for EnvironmentConfig {
    fn has_pending_fields(&self) -> bool {
        self.setup.command.has_pending_fields() || self.teardown.has_pending_fields()
    }

    fn try_resolve(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
        errs: &mut Vec<templating::Error>,
    ) {
        self.try_resolve_setup(path, values, errs);
        self.try_resolve_teardown(path, values, errs);
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SetupSection {
    #[serde(flatten)]
    pub command: CommandSection,
    #[serde(default)]
    pub provides: Vec<ValueDefinition>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers::file::{FileProvider, LocalFile, NamedFileProvider},
        templating::Field,
    };
    use simple_test_case::{dir_cases, test_case};
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

    #[dir_cases("crates/rtf-config/resources/config-tests/environment/valid")]
    #[test]
    fn valid_config(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");

        let res: serde_yaml::Result<EnvironmentConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let ctx = Context::new(
            PathBuf::from("resources/config-tests/environment/valid")
                .canonicalize()
                .unwrap(),
        );

        let env_config = res.unwrap();
        let res = env_config.validate(&ctx);

        assert!(res.is_ok(), "failed to validate: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/environment/validation-failures")]
    #[test]
    fn validation_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "validation-errors");

        let res: serde_yaml::Result<EnvironmentConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let ctx = Context::new(
            PathBuf::from("resources/config-tests/environment/validation-failures")
                .canonicalize()
                .unwrap(),
        );

        let env_config = res.unwrap();
        let res = env_config.validate(&ctx);

        assert!(res.is_err(), "expected validation failures");
        let errs = res.unwrap_err();

        // Validation Errors are an ordered list of individual errors with a kind.
        // To avoid breaking these tests when the user facing error message for each error
        // is modified, we only assert on the Kind of each error, not the full message.
        let mut err_kinds = Vec::new();
        for err in errs.iter() {
            err_kinds.push(format!("{:?}", err.kind()));
        }
        let concatenated_errs = err_kinds.join("\n");

        assert_eq!(
            &concatenated_errs, expected,
            "wrong validation errors: {errs:?}"
        );
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/environment/parse-failures")]
    #[test]
    fn parse_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let res: serde_yaml::Result<EnvironmentConfig> = serde_yaml::from_str(config);

        assert!(res.is_err(), "expected invalid YAML, got: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/environment/valid-templates")]
    #[test]
    fn valid_templates(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let raw_expected = get_file(&arr, "after-templating");

        let mut env_config: EnvironmentConfig = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();
        let expected: EnvironmentConfig = serde_yaml::from_str(raw_expected).unwrap();

        assert!(env_config.has_pending_fields(), "fields should be pending");

        let mut errs = Vec::new();
        env_config.try_resolve(&mut Vec::new(), &values, &mut errs);

        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
        assert!(
            !env_config.has_pending_fields(),
            "fields should be resolved"
        );
        assert_eq!(env_config, expected);
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/environment/invalid-templates")]
    #[test]
    fn invalid_templates(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let raw_values = get_file(&arr, "values");
        let expected = get_file(&arr, "templating-errors");

        let mut env_config: EnvironmentConfig = serde_yaml::from_str(config).unwrap();
        let values: HashMap<String, Scalar> = serde_yaml::from_str(raw_values).unwrap();

        assert!(env_config.has_pending_fields(), "fields should be pending");

        let mut errs = Vec::new();
        env_config.try_resolve(&mut Vec::new(), &values, &mut errs);

        assert!(
            env_config.has_pending_fields(),
            "fields should still be pending"
        );

        let str_errs: Vec<&str> = errs.iter().map(|e| e.as_ref()).collect();

        assert_eq!(str_errs.join("\n"), expected.trim());
    }

    // Helpers for the following tests

    fn definitions_from(strs: &[&str]) -> Vec<ValueDefinition> {
        strs.iter()
            .map(|s| ValueDefinition {
                name: s.to_string(),
                description: s.to_string(),
            })
            .collect()
    }

    fn cmd_section(name: &str, var: &str) -> CommandSection {
        CommandSection {
            command: name.to_string(),
            env_vars: [(var.to_uppercase(), Field::Pending(var.to_string()))]
                .into_iter()
                .collect(),
            file_providers: vec![NamedFileProvider {
                name: format!("{name}.txt"),
                env_var: format!("{}_PATH", name.to_uppercase()),
                provider: FileProvider::LocalPath(LocalFile {
                    relative_path: Field::Pending(format!("{name}-path")),
                }),
            }],
        }
    }

    /// Construct a stub [EnvironmentConfig] with the specified value definitions in the top level
    /// values section and setup.provides section.
    fn config_for_try_resolve_tests(
        available_vals: &[&str],
        provides: &[&str],
    ) -> EnvironmentConfig {
        EnvironmentConfig {
            name: String::new(),
            description: String::new(),
            values: definitions_from(available_vals),
            setup: SetupSection {
                command: cmd_section("setup", "foo"),
                provides: definitions_from(provides),
            },
            teardown: cmd_section("teardown", "bar"),
        }
    }

    #[test_case(&["foo", "setup-path"], &[], &[]; "both defined")]
    #[test_case(&[], &[], &["foo", "setup-path"]; "neither defined")]
    #[test_case(&["foo"], &[], &["setup-path"]; "foo defined")]
    #[test_case(&["setup-path"], &[], &["foo"]; "setup-path defined")]
    // This one is a little odd, but if a user tries to make use of a value that is coming from
    // setup.provides inside of setup itself that should still be an error as we need to template
    // before running the command.
    #[test_case(&[], &["foo", "setup-path"], &["foo", "setup-path"]; "defined in provides")]
    #[test]
    fn try_resolve_setup_respects_available_values(
        available_vals: &[&str],
        provides: &[&str],
        expected_unknown: &[&str],
    ) {
        let mut config = config_for_try_resolve_tests(available_vals, provides);

        // Both required values are available in the provided values map but they shouldn't be
        // usable unless they are defined.
        let values: HashMap<String, Scalar> = [("foo", "a"), ("setup-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let mut errs = Vec::new();
        config.try_resolve_setup(&mut Vec::new(), &values, &mut errs);

        let expected: Vec<templating::Error> = expected_unknown
            .iter()
            .map(|s| templating::Error::UnknownValue {
                value: s.to_string(),
            })
            .collect();

        assert_eq!(errs, expected);
    }

    #[test_case(&["bar", "teardown-path"], &[], &[]; "both defined at top level")]
    #[test_case(&[], &["bar", "teardown-path"], &[]; "both defined in setup provides")]
    #[test_case(&[], &[], &["bar", "teardown-path"]; "neither defined")]
    #[test_case(&["bar"], &[], &["teardown-path"]; "bar defined at top level")]
    #[test_case(&[], &["bar"], &["teardown-path"]; "bar defined in setup provides")]
    #[test_case(&["teardown-path"], &[], &["bar"]; "teardown-path defined at top level")]
    #[test_case(&[], &["teardown-path"], &["bar"]; "teardown-path defined in setup provides")]
    #[test]
    fn try_resolve_teardown_respects_available_values(
        available_vals: &[&str],
        provides: &[&str],
        expected_unknown: &[&str],
    ) {
        let mut config = config_for_try_resolve_tests(available_vals, provides);

        // Both required values are available in the provided values map but they shouldn't be
        // usable unless they are defined.
        let values: HashMap<String, Scalar> = [("bar", "a"), ("teardown-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let mut errs = Vec::new();
        config.try_resolve_teardown(&mut Vec::new(), &values, &mut errs);

        let expected: Vec<templating::Error> = expected_unknown
            .iter()
            .map(|s| templating::Error::UnknownValue {
                value: s.to_string(),
            })
            .collect();

        assert_eq!(errs, expected);
    }
}
