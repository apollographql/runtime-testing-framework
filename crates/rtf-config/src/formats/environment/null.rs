use crate::{
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    inlining::{self, InlineMode, InlinedProvider},
    providers,
    run::{Provider, RunEnvironment, RunProviders, ValidateEnvironment},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, Visitor},
};
use std::{collections::HashMap, fmt, path::Path, pin::Pin};

/// A simple no-op environment that takes no actions and never errors
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct NullEnvironment {
    #[serde(deserialize_with = "true_bool")]
    #[template(skip)]
    pub skip: bool,
}

impl ValidateEnvironment for NullEnvironment {}

impl RunEnvironment for NullEnvironment {
    async fn execute_setup(
        &self,
        _name: &str,
        _out_dir: &Path,
        _ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        Ok("{}".to_string())
    }

    async fn execute_teardown(
        &self,
        _name: &str,
        _out_dir: &Path,
        _ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        Ok("{}".to_string())
    }
}

impl CheckArrayDuplicates for NullEnvironment {
    const BASE_PATH: &str = "null_environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        Vec::new()
    }
}

impl Check for NullEnvironment {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

impl RunProviders for NullEnvironment {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        Vec::new()
    }

    fn inline<'a>(
        &'a mut self,
        _mode: &'a InlineMode,
        _ctx: &'a impl ResolutionContext,
        _cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }
}

/// Helper for deserializing a bool only if it has a value of 'true'
fn true_bool<'de, D>(de: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    struct TrueVisitor;
    impl Visitor<'_> for TrueVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bool")
        }

        fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v {
                Ok(v)
            } else {
                Err(E::custom("only a value of 'true' is allowed"))
            }
        }
    }

    de.deserialize_bool(TrueVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skip_true_succeeds() {
        let res = serde_yaml::from_str::<'_, NullEnvironment>("skip: true");

        assert!(res.is_ok(), "expected OK, got {res:?}");
        assert!(res.unwrap().skip, "skip should be true")
    }

    #[test]
    fn parse_skip_false_errors() {
        let res = serde_yaml::from_str::<'_, NullEnvironment>("skip: false");

        assert!(res.is_err(), "expected error, got {res:?}");
    }
}
