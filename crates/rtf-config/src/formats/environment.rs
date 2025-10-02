//! Parsing of the environment provisioner config file format
use crate::{
    ValueDefinition,
    checks::{self, Check, duplicate_keys},
    context::ResolutionContext,
    formats::{Result, values_for_config_file},
    providers::{command::CommandSection, file::Source},
    templating::{self, Scalar, Template},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

/// # Environment Config
///
/// Configuration for preparing and cleaning up the test environment as part of a test plan.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct EnvironmentConfig {
    /// The name of this environment configuration
    pub name: String,
    /// A brief description of how this environment setup works
    pub description: String,
    /// Definitions for the required values for templating this environment
    #[serde(default)]
    pub values: Vec<ValueDefinition>,
    /// The command to execute to prepare the environment for executing the test scenario
    pub setup: SetupSection,
    /// The command to execute to clean up the environment after executing the test scenario
    pub teardown: CommandSection,
}

impl EnvironmentConfig {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Ok(serde_yaml::from_str(&content)?)
    }

    /// Try to template the setup [CommandSection].
    ///
    /// Setup is only allowed to reference values that are declared in the values section of this
    /// config file.
    pub fn try_template_setup(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let definitions = self.values.iter();
        let allowed_values = values_for_config_file(values, definitions);

        self.setup
            .command
            .try_template_nested(path, "setup", &allowed_values)
    }

    /// Try to template the teardown [CommandSection].
    ///
    /// Teardown is allowed to reference values that come from the output of setup in addition to
    /// the values decalered in the values section of this config file.
    pub fn try_template_teardown(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let definitions = self.values.iter().chain(self.setup.provides.iter());
        let allowed_values = values_for_config_file(values, definitions);

        self.teardown
            .try_template_nested(path, "teardown", &allowed_values)
    }
}

impl Template for EnvironmentConfig {
    fn has_pending_fields(&self) -> bool {
        self.setup.command.has_pending_fields() || self.teardown.has_pending_fields()
    }

    fn required_values(&self) -> Vec<String> {
        let mut vals = self.setup.command.required_values();
        vals.extend(self.teardown.required_values());

        vals
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        values: &HashMap<String, Scalar>,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.try_template_setup(path, values));
        errs.append(self.try_template_teardown(path, values));

        errs.into_result(())
    }
}

impl Check for EnvironmentConfig {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        src: &Source,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        // Check that hard coded values and the ones coming from setup.provides are unique
        let all_values = self.values.iter().chain(self.setup.provides.iter());
        let duplicates = duplicate_keys(all_values, |v| &v.name);
        if !duplicates.is_empty() {
            errs.push(
                checks::ErrorKind::DuplicateValueNames,
                duplicates.join("\n"),
                path,
            );
        }

        // Check that each command is valid in isolation
        errs.append(self.setup.command.try_check_nested(path, "setup", src, ctx));
        errs.append(self.teardown.try_check_nested(path, "teardown", src, ctx));

        errs.into_result(())
    }
}

/// # Setup Command Section
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct SetupSection {
    #[serde(flatten)]
    pub command: CommandSection,
    /// Additional templating values that will be provided through the output of this command
    #[serde(default)]
    pub provides: Vec<ValueDefinition>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        providers::{
            command::{CommandProvider, CommandSpec},
            file::{FileProvider, InlineFile, NamedFileProvider, RelativeFile},
        },
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

        let dir = PathBuf::from("resources/config-tests/environment/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir);

        let env_config = res.unwrap();
        let res = env_config.try_check(&mut vec!["environment".to_string()], &src, &ctx);

        assert!(res.is_ok(), "failed check: {res:?}");
    }

    #[dir_cases("crates/rtf-config/resources/config-tests/environment/check-failures")]
    #[test]
    fn check_failures(_path: &str, content: &str) {
        let arr = load_archive(content);
        let config = get_file(&arr, "config.yaml");
        let expected = get_file(&arr, "check-errors");

        let res: serde_yaml::Result<EnvironmentConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let dir = PathBuf::from("resources/config-tests/environment/check-failures")
            .canonicalize()
            .unwrap();
        let ctx = Context::new();
        let src = Source::local(dir);

        let env_config = res.unwrap();
        let res = env_config.try_check(&mut vec!["environment".to_string()], &src, &ctx);

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

        let res = env_config.try_template(&mut Vec::new(), &values);

        assert!(res.is_ok(), "expected no errors, got {res:?}");
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

        let res = env_config.try_template(&mut Vec::new(), &values);

        assert!(
            env_config.has_pending_fields(),
            "fields should still be pending"
        );

        let errs = res.unwrap_err().into_vec();
        let str_errs: Vec<String> = errs.iter().map(|e| format!("{:?}", e.kind)).collect();

        assert_eq!(str_errs.join("\n"), expected.trim());
    }

    // Helpers for the following tests

    fn definitions_from(strs: &[&str]) -> Vec<ValueDefinition> {
        strs.iter()
            .map(|s| ValueDefinition {
                name: s.to_string(),
                description: s.to_string(),
                default: None,
            })
            .collect()
    }

    fn cmd_section(name: &str, var: &str) -> CommandSection {
        CommandSection {
            command: CommandSpec {
                name: "command.sh".to_string(),
                command_provider: CommandProvider::Inline(InlineFile {
                    content: "command".to_string(),
                }),
                args: Vec::new(),
            },
            env_vars: [(var.to_uppercase(), Field::Pending(var.to_string()))]
                .into_iter()
                .collect(),
            file_providers: vec![NamedFileProvider {
                name: format!("{name}.txt"),
                env_var: format!("{}_PATH", name.to_uppercase()),
                provider: FileProvider::RelativePath(RelativeFile {
                    path: Field::Pending(format!("{name}-path")),
                    src: None,
                }),
            }],
        }
    }

    /// Construct a stub [EnvironmentConfig] with the specified value definitions in the top level
    /// values section and setup.provides section.
    fn config_for_try_template_tests(
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
    #[test_case(
        &[], &[],
        &[
            ("foo", "setup.env_vars.FOO"),
            ("setup-path", "setup.file_providers.SETUP_PATH.path")
        ];
        "neither defined"
    )]
    #[test_case(
        &["foo"], &[],
        &[("setup-path", "setup.file_providers.SETUP_PATH.path")];
        "foo defined"
    )]
    #[test_case(
        &["setup-path"], &[],
        &[("foo", "setup.env_vars.FOO")];
        "setup-path defined"
    )]
    // This one is a little odd, but if a user tries to make use of a value that is coming from
    // setup.provides inside of setup itself that should still be an error as we need to template
    // before running the command.
    #[test_case(
            &[], &["foo", "setup-path"],
            &[
                ("foo", "setup.env_vars.FOO"),
                ("setup-path", "setup.file_providers.SETUP_PATH.path")
            ];
            "defined in provides"
        )]
    #[test]
    fn try_template_setup_respects_available_values(
        available_vals: &[&str],
        provides: &[&str],
        expected_unknown: &[(&str, &str)],
    ) {
        let mut config = config_for_try_template_tests(available_vals, provides);

        // Both required values are available in the provided values map but they shouldn't be
        // usable unless they are defined.
        let values: HashMap<String, Scalar> = [("foo", "a"), ("setup-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let res = config.try_template_setup(&mut Vec::new(), &values);
        let errs = match res {
            Ok(_) => Vec::new(),
            Err(e) => e.into_vec(),
        };

        let expected: Vec<templating::Error> = expected_unknown
            .iter()
            .map(|(message, path)| templating::Error {
                kind: templating::ErrorKind::UnknownValue,
                message: message.to_string(),
                path: path.to_string(),
            })
            .collect();

        assert_eq!(errs, expected);
    }

    #[test_case(&["bar", "teardown-path"], &[], &[]; "both defined at top level")]
    #[test_case(&[], &["bar", "teardown-path"], &[]; "both defined in setup provides")]
    #[test_case(
            &[], &[],
            &[
                ("bar", "teardown.env_vars.BAR"),
                ("teardown-path", "teardown.file_providers.TEARDOWN_PATH.path")
            ];
            "neither defined"
        )]
    #[test_case(
            &["bar"], &[],
            &[("teardown-path", "teardown.file_providers.TEARDOWN_PATH.path")];
            "bar defined at top level"
        )]
    #[test_case(
            &[], &["bar"],
            &[("teardown-path", "teardown.file_providers.TEARDOWN_PATH.path")];
            "bar defined in setup provides"
        )]
    #[test_case(
            &["teardown-path"], &[],
            &[("bar", "teardown.env_vars.BAR")];
            "teardown-path defined at top level"
        )]
    #[test_case(
            &[], &["teardown-path"],
            &[("bar", "teardown.env_vars.BAR")];
            "teardown-path defined in setup provides"
        )]
    #[test]
    fn try_template_teardown_respects_available_values(
        available_vals: &[&str],
        provides: &[&str],
        expected_unknown: &[(&str, &str)],
    ) {
        let mut config = config_for_try_template_tests(available_vals, provides);

        // Both required values are available in the provided values map but they shouldn't be
        // usable unless they are defined.
        let values: HashMap<String, Scalar> = [("bar", "a"), ("teardown-path", "b")]
            .iter()
            .map(|(k, v)| (k.to_string(), Scalar::String(v.to_string())))
            .collect();

        let res = config.try_template_teardown(&mut Vec::new(), &values);
        let errs = match res {
            Ok(_) => Vec::new(),
            Err(e) => e.into_vec(),
        };

        let expected: Vec<templating::Error> = expected_unknown
            .iter()
            .map(|(message, path)| templating::Error {
                kind: templating::ErrorKind::UnknownValue,
                message: message.to_string(),
                path: path.to_string(),
            })
            .collect();

        assert_eq!(errs, expected);
    }
}
