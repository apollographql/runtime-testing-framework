//! An Orchestrator specific test plan implementation
use rtf_config::{
    Prepare,
    checks::{Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    enum_impl_check,
    formats::{
        DockerComposeEnvironment, DockerScenario, FileProviderServices, K8sEnvironment,
        NullEnvironment, PrometheusQuery, TestPlan,
    },
    inlining::{self, Inline, InlineMode, InlinedProvider, ProviderCacheKey},
    run::{Provider, RunProviders, ValidateEnvironment},
    templating::Template as _,
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, pin::Pin};

/// Test plan restricted to Orchestrator compatible scenarios and environments
pub type OrchestratorTestPlan = TestPlan<Orchestrator>;

/// Marker for the Orchestrator test plan variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Orchestrator;

impl Prepare for Orchestrator {
    type Scenario = DockerScenario;
    type Environment = OrchestratorEnvironment;
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(
    untagged,
    expecting = "expected null or docker-compose environment when running via the Orchestrator"
)]
#[allow(clippy::large_enum_variant)] // We only ever allocate one of these, not multiples
pub enum OrchestratorEnvironment {
    // Null needs to be the first variant in this enum to ensure that any time `skip: true` is
    // set, we resolve to a NullEnvironment.
    Null(NullEnvironment),
    DockerCompose(DockerComposeEnvironment),
    K8s(K8sEnvironment),
}

impl OrchestratorEnvironment {
    pub fn prometheus_queries(&self) -> &[PrometheusQuery] {
        match self {
            Self::Null(_) => &[],
            Self::DockerCompose(inner) => &inner.output_collection.prometheus,
            Self::K8s(inner) => &inner.output_collection.prometheus,
        }
    }

    pub async fn inline_manifest_files(
        &mut self,
        ctx: &impl ResolutionContext,
        cache: &mut HashMap<ProviderCacheKey, InlinedProvider>,
    ) -> inlining::Result<()> {
        match self {
            Self::Null(_) => Ok(()),
            Self::DockerCompose(inner) => inner.inline_manifests(ctx, cache).await,
            Self::K8s(inner) => inner.inline_manifests(ctx, cache).await,
        }
    }

    pub fn file_provider_services_from_inline(&self) -> FileProviderServices {
        match self {
            Self::Null(_) | Self::K8s(_) => FileProviderServices::default(),
            Self::DockerCompose(inner) => FileProviderServices::from_inline(inner),
        }
    }

    /// The template variables that this environment's compose file providers depend on (if any).
    pub fn manifest_template_variables(&self) -> Vec<String> {
        match self {
            Self::Null(_) => Vec::new(),
            Self::DockerCompose(inner) => inner
                .resources
                .compose_files
                .iter()
                .flat_map(|ncfp| ncfp.required_variables())
                .collect(),
            Self::K8s(inner) => inner
                .resources
                .resources
                .iter()
                .flat_map(|ncfp| ncfp.required_variables())
                .collect(),
        }
    }
}

enum_impl_check!(OrchestratorEnvironment => Null, DockerCompose, K8s);

impl ValidateEnvironment for OrchestratorEnvironment {}

impl RunProviders for OrchestratorEnvironment {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        match self {
            Self::Null(inner) => inner.named_providers(),
            Self::DockerCompose(inner) => inner.named_providers(),
            Self::K8s(inner) => inner.named_providers(),
        }
    }
}

impl Inline for OrchestratorEnvironment {
    fn try_inline<'a>(
        &'a mut self,
        mode: InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<ProviderCacheKey, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        match self {
            Self::Null(inner) => inner.try_inline(mode, ctx, cache),
            Self::DockerCompose(inner) => inner.try_inline(mode, ctx, cache),
            Self::K8s(inner) => inner.try_inline(mode, ctx, cache),
        }
    }
}

impl CheckArrayDuplicates for OrchestratorEnvironment {
    const BASE_PATH: &str = "environment_execution";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        match self {
            Self::Null(inner) => inner.deduplicated_arrays(),
            Self::DockerCompose(inner) => inner.deduplicated_arrays(),
            Self::K8s(inner) => inner.deduplicated_arrays(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rtf_config::{context::Context, formats::ComposeResources};

    fn null_env() -> OrchestratorEnvironment {
        OrchestratorEnvironment::Null(NullEnvironment { skip: true })
    }

    fn docker_compose_env() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: vec![],
            },
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: Default::default(),
        }
    }

    #[test]
    fn null_environment_round_trips() {
        let env = null_env();
        let yaml = yaml_serde::to_string(&env).expect("null environment should serialize");
        let parsed: OrchestratorEnvironment =
            yaml_serde::from_str(&yaml).expect("null environment should round-trip");

        assert_eq!(parsed, env);
    }

    #[test]
    fn docker_compose_environment_round_trips() {
        let env = OrchestratorEnvironment::DockerCompose(docker_compose_env());
        let yaml =
            yaml_serde::to_string(&env).expect("docker-compose environment should serialize");
        let parsed: OrchestratorEnvironment =
            yaml_serde::from_str(&yaml).expect("docker-compose environment should round-trip");

        assert_eq!(parsed, env);
    }

    #[test]
    fn script_shaped_yaml_fails_to_deserialize() {
        // Shaped like a rtf-config ScriptEnvironment - has neither `skip` nor `compose_files`,
        // so it should match neither OrchestratorEnvironment variant.
        let yaml = r#"
setup:
  command:
    name: setup.sh
    kind: inline
    content: "echo hi"
teardown:
  command:
    name: teardown.sh
    kind: inline
    content: "echo bye"
"#;

        let res = yaml_serde::from_str::<OrchestratorEnvironment>(yaml);
        assert!(res.is_err(), "expected a script environment to be rejected");

        let err = res.unwrap_err().to_string();
        assert!(
            err.contains(
                "expected null or docker-compose environment when running via the Orchestrator"
            ),
            "expected the custom untagged-enum message, got: {err}"
        );
    }

    #[test]
    fn prometheus_queries_empty_for_null_environment() {
        assert!(null_env().prometheus_queries().is_empty());
    }

    #[test]
    fn prometheus_queries_returns_docker_compose_queries() {
        let mut dce = docker_compose_env();
        dce.output_collection.prometheus.push(PrometheusQuery {
            name: "up".to_string(),
            step: "15s".to_string(),
            query: "up".to_string(),
        });
        let env = OrchestratorEnvironment::DockerCompose(dce);

        assert_eq!(env.prometheus_queries().len(), 1);
    }

    #[test]
    fn file_provider_services_from_inline_default_for_null_environment() {
        assert_eq!(
            null_env().file_provider_services_from_inline(),
            FileProviderServices::default()
        );
    }

    #[test]
    fn check_succeeds_for_null_environment() {
        let ctx = Context::new();
        let res = null_env().try_check(&mut Vec::new(), &ctx);

        assert!(res.is_ok(), "expected Check to succeed, got {res:?}");
    }

    fn docker_compose_env_with(providers: &[&str]) -> DockerComposeEnvironment {
        let compose_files = providers
            .iter()
            .map(|yaml| {
                yaml_serde::from_str(yaml).expect("compose file provider should deserialize")
            })
            .collect();

        let mut env = docker_compose_env();
        env.resources.compose_files = compose_files;

        env
    }

    fn compose_env_with(providers: &[&str]) -> OrchestratorEnvironment {
        OrchestratorEnvironment::DockerCompose(docker_compose_env_with(providers))
    }

    #[test]
    fn compose_template_variables_reports_pending_compose_path_variables() {
        const TEMPLATED_COMPOSE_PATH: &str = indoc!(
            r#"
            name: compose.yaml
            kind: relative_path
            path: "{{ compose_dir }}"
        "#
        );

        assert_eq!(
            compose_env_with(&[TEMPLATED_COMPOSE_PATH]).manifest_template_variables(),
            vec!["compose_dir".to_string()]
        );
    }
}
