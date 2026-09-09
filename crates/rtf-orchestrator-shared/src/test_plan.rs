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
    inlining::{self, InlineMode, InlinedProvider},
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
        cache: &mut HashMap<u64, InlinedProvider>,
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

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        match self {
            Self::Null(inner) => inner.inline(mode, ctx, cache),
            Self::DockerCompose(inner) => inner.inline(mode, ctx, cache),
            Self::K8s(inner) => inner.inline(mode, ctx, cache),
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
    use rtf_config::{
        context::Context,
        formats::{ComposeResources, EnvironmentService, ServiceReplicas},
        providers,
    };
    use std::assert_matches;

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
        let yaml = serde_yaml::to_string(&env).expect("null environment should serialize");
        let parsed: OrchestratorEnvironment =
            serde_yaml::from_str(&yaml).expect("null environment should round-trip");

        assert_eq!(parsed, env);
    }

    #[test]
    fn docker_compose_environment_round_trips() {
        let env = OrchestratorEnvironment::DockerCompose(docker_compose_env());
        let yaml =
            serde_yaml::to_string(&env).expect("docker-compose environment should serialize");
        let parsed: OrchestratorEnvironment =
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

        let res = serde_yaml::from_str::<OrchestratorEnvironment>(yaml);
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
                serde_yaml::from_str(yaml).expect("compose file provider should deserialize")
            })
            .collect();

        let mut env = docker_compose_env();
        env.resources.compose_files = compose_files;

        env
    }

    fn compose_env_with(providers: &[&str]) -> OrchestratorEnvironment {
        OrchestratorEnvironment::DockerCompose(docker_compose_env_with(providers))
    }

    fn services_of(providers: &[&str]) -> Vec<EnvironmentService> {
        docker_compose_env_with(providers)
            .services()
            .expect("compose files in these tests are already inline")
    }

    fn service(name: &str, image: Option<&str>, replicas: ServiceReplicas) -> EnvironmentService {
        EnvironmentService {
            name: name.to_string(),
            image: image.map(String::from),
            replicas,
        }
    }

    /// One fixture covering every shape the extraction logic has to handle: a bare image, `${VAR}`
    /// placeholders in both image and replica count, `deploy.replicas` winning over `scale`, a
    /// quoted count, a count that is not a valid `u32`, and a service with no image at all.
    const ALL_SERVICE_SHAPES: &str = indoc!(
        r#"
        name: compose.yaml
        kind: inline
        content: |
          services:
            default-replicas:
              image: nginx:1.25
            templated:
              image: ${ROUTER_IMAGE}:${ROUTER_TAG}
              deploy:
                replicas: ${ROUTER_REPLICAS}
            fixed-replicas:
              image: fixed:1
              deploy:
                replicas: 3
            quoted-replicas:
              image: quoted:1
              deploy:
                replicas: "2"
            scaled:
              image: scaled:1
              scale: 4
            deploy-wins:
              image: both:1
              scale: 9
              deploy:
                replicas: 5
            invalid-replicas:
              image: invalid:1
              deploy:
                replicas: -1
            no-image:
              build: .
        "#
    );

    #[test]
    fn services_extracts_every_service_declaration() {
        use ServiceReplicas::{Fixed, Variable};

        assert_eq!(
            services_of(&[ALL_SERVICE_SHAPES]),
            vec![
                service("default-replicas", Some("nginx:1.25"), Fixed(1)),
                service("deploy-wins", Some("both:1"), Fixed(5)),
                service("fixed-replicas", Some("fixed:1"), Fixed(3)),
                service("invalid-replicas", Some("invalid:1"), Fixed(1)),
                service("no-image", None, Fixed(1)),
                service("quoted-replicas", Some("quoted:1"), Fixed(2)),
                service("scaled", Some("scaled:1"), Fixed(4)),
                service(
                    "templated",
                    Some("${ROUTER_IMAGE}:${ROUTER_TAG}"),
                    Variable("${ROUTER_REPLICAS}".to_string())
                ),
            ]
        );
    }

    #[test]
    fn services_merges_field_wise_across_compose_files() {
        const BASE_COMPOSE: &str = indoc!(
            r#"
            name: base.yaml
            kind: inline
            content: |
              services:
                web:
                  image: nginx:1.25
                  deploy:
                    replicas: 1
                worker:
                  image: worker:v1
            "#
        );

        /// Mentions `web` only to scale it up, never repeating its image.
        const OVERLAY_COMPOSE: &str = indoc!(
            r#"
            name: overlay.yaml
            kind: inline
            content: |
              services:
                web:
                  deploy:
                    replicas: 4
            "#
        );

        assert_eq!(
            services_of(&[BASE_COMPOSE, OVERLAY_COMPOSE]),
            vec![
                service("web", Some("nginx:1.25"), ServiceReplicas::Fixed(4)),
                service("worker", Some("worker:v1"), ServiceReplicas::Fixed(1)),
            ]
        );
    }

    #[test]
    fn services_reads_every_file_in_an_inline_dir() {
        const COMPOSE_DIR: &str = indoc!(
            r#"
            name: compose-dir
            kind: inline_dir
            files:
              - path: base.yaml
                content: |
                  services:
                    a:
                      image: a:1
              - path: overlay.yaml
                content: |
                  services:
                    b:
                      image: b:1
            "#
        );
        assert_eq!(
            services_of(&[COMPOSE_DIR]),
            vec![
                service("a", Some("a:1"), ServiceReplicas::Fixed(1)),
                service("b", Some("b:1"), ServiceReplicas::Fixed(1)),
            ]
        );
    }

    #[test]
    fn services_is_empty_when_no_services_can_be_read() {
        const NO_SERVICES_COMPOSE: &str = indoc!(
            r#"
            name: compose.yaml
            kind: inline
            content: |
              version: '3'
            "#
        );

        assert!(services_of(&[NO_SERVICES_COMPOSE]).is_empty());
    }

    #[test]
    fn services_errors_on_a_provider_that_was_not_inlined() {
        const REQUIRED_COMPOSE: &str = indoc!(
            r#"
            name: required-compose
            kind: required
            message: must provide a compose file
            "#
        );

        let res = docker_compose_env_with(&[REQUIRED_COMPOSE]).services();

        assert_matches!(
            res,
            Err(providers::Error::ManifestFileNotInlined { ref name }) if name == "required-compose",
            "expected ComposeFileNotInlined, got {res:?}"
        );
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
