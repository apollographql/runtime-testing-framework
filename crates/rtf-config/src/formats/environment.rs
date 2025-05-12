//! Parsing of the environment provisioner config file format
use crate::{
    ValueSchema,
    formats::{Error, Result},
    providers::{
        CommandProvider, Context,
        file::{FileProvider, IntoUtf8FileContent},
    },
};
use futures::future::join_all;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

// TODO: need to implement injection of values into the config file before
// -> look at https://crates.io/crates/handlebars

/// Resolved and validated config
#[derive(Debug, Clone, PartialEq)]
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
        raw.validate(&ctx)?;

        raw.try_resolve(&ctx).await
    }
}

/// The raw serialization format for parsing user provided config
#[derive(Debug, Clone, Deserialize)]
pub struct RawEnvironmentConfig {
    pub name: String,
    pub setup_command: CommandProvider,
    pub values: Vec<ValueSchema>,
    pub parameters: Map<String, Value>,
    pub file_parameters: Vec<FileParam>,
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

    pub fn validate(&self, ctx: &Context) -> Result<()> {
        let mut errs = Vec::new();

        for param in self.file_parameters.iter() {
            if let Err(e) = param.provider.validate(ctx) {
                errs.push(format!("file param {:?}: {e}", param.name));
            }
        }

        // TODO: validate that no file param names clash with names in raw params and that all
        // file param names are unique

        // TODO: all value schemas need to be checked to see if they are actually valid JSON-schema
        // schemas

        if errs.is_empty() {
            Ok(())
        } else {
            Err(Error::InvalidFileProviders { errs })
        }
    }

    /// Assume that we have already validated this config and attempt to run all providers in order
    /// to generate a fully resolved [EnvironmentConfig].
    pub async fn try_resolve(self, ctx: &Context) -> Result<EnvironmentConfig> {
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

#[derive(Debug, Clone, Deserialize)]
pub struct FileParam {
    pub name: String,
    #[serde(flatten)]
    pub provider: FileProvider,
}

impl FileParam {
    async fn try_into_file_name_and_content(self, ctx: &Context) -> (String, Result<String>) {
        let res = self.provider.try_into_file_content(ctx).await;

        (self.name, res.map_err(Into::into))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use simple_test_case::dir_cases;
    use std::path::PathBuf;

    #[dir_cases("crates/rtf-config/resources/env-config-tests/valid")]
    #[tokio::test]
    async fn valid_environment_config_parses_and_resolves(_path: &str, content: &str) {
        let raw: RawEnvironmentConfig = serde_yaml::from_str(content).expect("to parse with serde");
        let p = PathBuf::from("resources/env-config-tests/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);

        let res = raw.validate(&ctx);
        assert!(res.is_ok(), "failed to validate: {res:?}");

        let res = raw.try_resolve(&ctx).await;
        assert!(res.is_ok(), "failed to resolve: {res:?}");
    }

    #[tokio::test]
    async fn minimal_env_config_resolves_correctly() {
        let content = include_str!("../../resources/env-config-tests/valid/minimal.yaml");
        let raw: RawEnvironmentConfig = serde_yaml::from_str(content).expect("to parse with serde");
        let p = PathBuf::from("resources/env-config-tests/valid")
            .canonicalize()
            .unwrap();
        let ctx = Context::new(p);

        let res = raw.validate(&ctx);
        assert!(res.is_ok(), "failed to validate: {res:?}");

        let res = raw.try_resolve(&ctx).await;
        assert!(res.is_ok(), "failed to resolve: {res:?}");

        let cfg = res.unwrap();

        let expected = EnvironmentConfig {
            name: "minimal".to_string(),
            setup_command: CommandProvider::Local {
                local: "../../scripts/deploy-router.sh".to_string(),
            },
            values: vec![ValueSchema {
                name: "router_build".to_string(),
                description: "How to fetch or build the desired version of the Router".to_string(),
                schema: None,
            }],
            parameters: [
                ("foo".to_string(), Value::String("bar".to_string())),
                (
                    "my-file.txt".to_string(),
                    Value::String("My inline file content.\n".to_string()),
                ),
            ]
            .into_iter()
            .collect(),
            provides: vec![ValueSchema {
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
