use crate::{
    Source,
    checks::{self, Check},
    context::ResolutionContext,
    providers::{self, file::AsUtf8FileContent},
    templating::{Field, Scalar},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// # Custom Provider
///
/// Use a custom provider to execute a command and produce a set of files.
///
/// ```yaml
/// - name: "router-docker-compose"
///   env_var: ROUTER_DOCKER_COMPOSE
///   kind: custom_provider
///   type: "router-docker-compose"
///   graph_ref: "graph@variant"
///   router_version: "v2.x.y"
///   build_router_from_source: "false"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct CustomProvider {
    /// The type of custom provider to use. This is the name of the custom provider to use.
    #[serde(rename = "type")]
    #[template(skip)]
    pub(crate) ty: String,

    /// The arguments to pass to the custom provider.
    #[serde(flatten)]
    pub(crate) arguments: HashMap<String, Field<Scalar>>,

    /// Set during TestPlan parsing as part of overrides and templating.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    #[template(skip)]
    #[doc(hidden)]
    pub(crate) src: Option<Source>,
}

// AsUtf8FileContent needs to be implemented for CustomProvider to be a valid FileProvider that can be added to the NamedFileProvider enum
// However, the AsUtf8FileContent methods should never actually be called
impl AsUtf8FileContent for CustomProvider {
    async fn try_get_file_content(
        &self,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        panic!(
            "Should not be able to get here. Custom provider should have been expanded when templating the config."
        )
    }
}

// Check needs to be implemented for CustomProvider to be a valid FileProvider that can be added to the NamedFileProvider enum
// However, the Check methods should never actually be called
impl Check for CustomProvider {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        panic!(
            "Should not be able to get here. Custom provider should have been expanded when templating the config."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use indoc::indoc;

    const CUSTOM_PROVIDER_YAML: &str = indoc!(
        r#"
        type: "my-custom-provider"
        key1: "{{ value1 }}"
        key2: "value2"
        key3: "value3"
        "#
    );

    // I've included this test as a sanity check while we have not added CustomProvider to the NamedFileProviders.
    // Once we have added CustomProvider to the NamedFileProviders, we can remove this test and add a test for the
    // CustomProvider to the mod.rs tests for parsing all the NamedFileProviders.
    #[test]
    fn parse_custom_provider_success() {
        let config: CustomProvider = serde_yaml::from_str(CUSTOM_PROVIDER_YAML).unwrap();
        assert_eq!(config.ty, "my-custom-provider");
        assert_eq!(config.arguments.len(), 3);
        assert_eq!(
            config.arguments.get("key1").unwrap(),
            &Field::Pending("value1".to_string())
        );
        assert_eq!(
            config
                .arguments
                .get("key2")
                .unwrap()
                .as_resolved()
                .to_string(),
            "value2".to_string()
        );
        assert_eq!(
            config
                .arguments
                .get("key3")
                .unwrap()
                .as_resolved()
                .to_string(),
            "value3".to_string()
        );
    }

    #[test]
    #[should_panic(
        expected = "Should not be able to get here. Custom provider should have been expanded when templating the config."
    )]
    fn custom_provider_check_is_missing() {
        let custom_provider = CustomProvider {
            ty: "my-custom-provider".to_string(),
            arguments: HashMap::new(),
            src: None,
        };

        let ctx = Context::new();

        let _res = custom_provider.try_check(&mut Vec::new(), &ctx);
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Custom provider should have been expanded when templating the config."
    )]
    async fn custom_provider_try_get_file_content_panics() {
        let custom_provider = CustomProvider {
            ty: "my-custom-provider".to_string(),
            arguments: HashMap::new(),
            src: None,
        };

        let ctx = Context::new();

        let _res = custom_provider.try_get_file_content(&ctx).await;
    }
}
