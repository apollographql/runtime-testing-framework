//! Parsing of the environment provisioner config file format
use crate::{
    ValueSchema,
    formats::{Error, Result},
    providers::{
        Context,
        command::CommandProvider,
        file::{IntoUtf8FileContent, NamedFileProvider},
    },
    validation,
};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

// TODO: need to implement injection of values into the config file before
// -> look at https://crates.io/crates/handlebars

/// Resolved and validated config
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnvironmentConfig {
    pub name: String,
    pub setup_command: CommandProvider,
    pub values: Vec<ValueSchema>,
    pub parameters: Map<String, Value>,
    pub provides: Vec<ValueSchema>,
}

impl EnvironmentConfig {
    pub async fn try_load_and_resolve(p: impl Into<PathBuf>) -> Result<Self> {
        let p = p.into();
        let raw = RawEnvironmentConfig::try_load_from_path(&p)?;
        let ctx = Context::new(p);

        raw.try_validate_and_resolve(&ctx).await
    }
}

/// The raw serialization format for parsing user provided config
#[derive(Debug, Clone, Deserialize)]
pub struct RawEnvironmentConfig {
    pub name: String,
    pub setup_command: CommandProvider,
    pub values: Vec<ValueSchema>,
    pub parameters: Map<String, Value>,
    pub file_parameters: Vec<NamedFileProvider>,
    pub provides: Vec<ValueSchema>,
}

impl FromStr for RawEnvironmentConfig {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let raw: Self = serde_yaml::from_str(s)?;

        Ok(raw)
    }
}

impl RawEnvironmentConfig {
    pub fn try_load_from_path(p: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(p)?;

        Self::from_str(&content)
    }

    pub fn validate(&self, ctx: &Context) -> validation::Result<()> {
        let mut errs = validation::ErrorBuilder::new();

        for param in self.file_parameters.iter() {
            if let Err(e) = param.provider.validate(ctx) {
                errs.extend_with_prefix(e, format!("file param {:?}:", param.name));
            }
        }

        // TODO: validate that no file param names clash with names in raw params and that all
        // file param names are unique

        // TODO: all value schemas need to be checked to see if they are actually valid JSON-schema
        // schemas

        errs.into_result(())
    }

    /// [Validate][Self::validate] this config file before attempting to resolve all of the
    /// providers it contains in order to obtain the fully resolved [EnvironmentConfig].
    pub async fn try_validate_and_resolve(self, ctx: &Context) -> Result<EnvironmentConfig> {
        self.validate(ctx)?;

        self.try_resolve(ctx).await
    }

    /// Attempt to resolve all of the providers contained in this config without running their
    /// required validation.
    ///
    /// # Panics
    /// Some [FileProvider][0] implementations can panic if their `try_into_file_name` method is
    /// called without fist checking the it is valid to do so ([RequiredFile][1] for example).
    ///
    ///   [0]: crate::providers::file::FileProvider
    ///   [1]: crate::providers::file::RequiredFile
    async fn try_resolve(self, ctx: &Context) -> Result<EnvironmentConfig> {
        let mut cfg = EnvironmentConfig {
            name: self.name,
            setup_command: self.setup_command,
            values: self.values,
            parameters: self.parameters,
            provides: self.provides,
        };

        let futs = self
            .file_parameters
            .into_iter()
            .map(|param| param.try_into_file_name_and_content(ctx));

        // TODO: do we want to batch process these?
        let resolved_providers = join_all(futs).await;
        let mut errs = Vec::new();

        for (fname, res) in resolved_providers.into_iter() {
            match res {
                Ok(content) => {
                    cfg.parameters.insert(fname, Value::String(content));
                }
                Err(e) => errs.push(format!("file param {fname:?}: {e}")),
            }
        }

        if errs.is_empty() {
            Ok(cfg)
        } else {
            Err(Error::FailedFileProviders { errs })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtf_test_utils;
    use simple_test_case::dir_cases;
    use simple_txtar::Archive;
    use std::path::PathBuf;

    #[dir_cases("crates/rtf-config/resources/env-config-tests")]
    #[tokio::test]
    async fn environment_config_scenarios(_path: &str, content: &str) {
        let p = PathBuf::from("resources/env-config-tests")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);

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

        let res: serde_yaml::Result<RawEnvironmentConfig> = serde_yaml::from_str(config);
        assert!(res.is_ok(), "{res:?}");

        let raw = res.unwrap();

        let res = raw.validate(&ctx);
        assert!(res.is_ok(), "failed to validate: {res:?}");

        let res = raw.try_validate_and_resolve(&ctx).await;
        assert!(res.is_ok(), "failed to resolve: {res:?}");

        let resolved_config = res.unwrap();
        let res = rtf_test_utils::to_pretty_json_with_indent(&resolved_config, 4);

        if let Some(expected) = expected_json {
            assert_eq!(res, expected.content.trim());
        }
    }
}
