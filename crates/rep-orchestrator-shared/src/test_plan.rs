//! An Orchestrator specific test plan implementation
use rtf_config::{
    Execution,
    checks::{Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    enum_impl_check,
    formats::{
        DockerComposeEnvironment, DockerScenario, FileProviderServices, NullEnvironment,
        PrometheusQuery, TestPlan,
    },
    inlining::{self, InlineMode, InlinedProvider},
    providers,
    run::{Provider, RunEnvironment, RunProviders},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, pin::Pin};

/// Test plan restricted to Orchestrator compatible scenarios and environments
pub type RepTestPlan = TestPlan<Rep>;

/// Marker for the Orchestrator test plan variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Rep;

impl Execution for Rep {
    type Scenario = DockerScenario;
    type Environment = RepEnvironment;
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(
    untagged,
    expecting = "expected null or docker-compose environment when running via the Orchestrator"
)]
#[allow(clippy::large_enum_variant)] // We only ever allocate one of these, not multiples
pub enum RepEnvironment {
    // Null needs to be the first variant in this enum to ensure that any time `skip: true` is
    // set, we resolve to a NullEnvironment.
    Null(NullEnvironment),
    DockerCompose(DockerComposeEnvironment),
}

impl RepEnvironment {
    pub fn prometheus_queries(&self) -> &[PrometheusQuery] {
        match self {
            RepEnvironment::Null(_) => &[],
            RepEnvironment::DockerCompose(dce) => &dce.output_collection.prometheus,
        }
    }

    pub async fn inline_compose_files(
        &mut self,
        ctx: &impl ResolutionContext,
        cache: &mut HashMap<u64, InlinedProvider>,
    ) -> inlining::Result<()> {
        match self {
            RepEnvironment::Null(_) => Ok(()),
            RepEnvironment::DockerCompose(dce) => dce.inline_compose_files(ctx, cache).await,
        }
    }

    pub fn file_provider_services_from_inline(&self) -> FileProviderServices {
        match self {
            RepEnvironment::Null(_) => FileProviderServices::default(),
            RepEnvironment::DockerCompose(dce) => FileProviderServices::from_inline(dce),
        }
    }
}

enum_impl_check!(RepEnvironment => Null, DockerCompose);

impl RunEnvironment for RepEnvironment {
    async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        match self {
            RepEnvironment::Null(inner) => inner.execute_setup(name, out_dir, ctx).await,
            RepEnvironment::DockerCompose(inner) => inner.execute_setup(name, out_dir, ctx).await,
        }
    }

    async fn execute_teardown(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        match self {
            RepEnvironment::Null(inner) => inner.execute_teardown(name, out_dir, ctx).await,
            RepEnvironment::DockerCompose(inner) => {
                inner.execute_teardown(name, out_dir, ctx).await
            }
        }
    }
}

impl RunProviders for RepEnvironment {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        match self {
            RepEnvironment::Null(inner) => inner.named_providers(),
            RepEnvironment::DockerCompose(inner) => inner.named_providers(),
        }
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        match self {
            RepEnvironment::Null(inner) => inner.inline(mode, ctx, cache),
            RepEnvironment::DockerCompose(inner) => inner.inline(mode, ctx, cache),
        }
    }
}

impl CheckArrayDuplicates for RepEnvironment {
    const BASE_PATH: &str = "environment_execution";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        match self {
            RepEnvironment::Null(inner) => inner.deduplicated_arrays(),
            RepEnvironment::DockerCompose(inner) => inner.deduplicated_arrays(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtf_config::context::Context;

    fn null_env() -> RepEnvironment {
        RepEnvironment::Null(NullEnvironment { skip: true })
    }

    fn docker_compose_env() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: Default::default(),
        }
    }

    #[test]
    fn null_environment_round_trips() {
        let env = null_env();
        let yaml = serde_yaml::to_string(&env).expect("null environment should serialize");
        let parsed: RepEnvironment =
            serde_yaml::from_str(&yaml).expect("null environment should round-trip");

        assert_eq!(parsed, env);
    }

    #[test]
    fn docker_compose_environment_round_trips() {
        let env = RepEnvironment::DockerCompose(docker_compose_env());
        let yaml =
            serde_yaml::to_string(&env).expect("docker-compose environment should serialize");
        let parsed: RepEnvironment =
            serde_yaml::from_str(&yaml).expect("docker-compose environment should round-trip");

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

        let res = serde_yaml::from_str::<RepEnvironment>(yaml);
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
        let env = RepEnvironment::DockerCompose(dce);

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
}
