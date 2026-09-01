//! An Orchestrator specific test plan implementation
use rtf_config::{
    Prepare, Run,
    checks::{Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    enum_impl_check,
    formats::{
        DockerComposeEnvironment, DockerScenario, FileProviderServices, NullEnvironment,
        PrometheusQuery, TestPlan,
    },
    inlining::{self, InlineMode, InlinedProvider},
    providers,
    run::{Provider, RunEnvironment, RunProviders, ValidateEnvironment},
    templating::Template as _,
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    pin::Pin,
};

/// Test plan restricted to Orchestrator compatible scenarios and environments
pub type OrchestratorTestPlan = TestPlan<Orchestrator>;

/// Marker for the Orchestrator test plan variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Orchestrator;

impl Prepare for Orchestrator {
    type Scenario = DockerScenario;
    type Environment = OrchestratorEnvironment;
}

impl Run for Orchestrator {}

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
}

impl OrchestratorEnvironment {
    pub fn prometheus_queries(&self) -> &[PrometheusQuery] {
        match self {
            OrchestratorEnvironment::Null(_) => &[],
            OrchestratorEnvironment::DockerCompose(dce) => &dce.output_collection.prometheus,
        }
    }

    pub async fn inline_compose_files(
        &mut self,
        ctx: &impl ResolutionContext,
        cache: &mut HashMap<u64, InlinedProvider>,
    ) -> inlining::Result<()> {
        match self {
            OrchestratorEnvironment::Null(_) => Ok(()),
            OrchestratorEnvironment::DockerCompose(dce) => {
                dce.inline_compose_files(ctx, cache).await
            }
        }
    }

    pub fn file_provider_services_from_inline(&self) -> FileProviderServices {
        match self {
            OrchestratorEnvironment::Null(_) => FileProviderServices::default(),
            OrchestratorEnvironment::DockerCompose(dce) => FileProviderServices::from_inline(dce),
        }
    }

    /// Requires that only inline providers are present, erroring if any non-inline providers are
    /// encountered.
    /// [inline_compose_files](OrchestratorEnvironment::inline_compose_files) can be used to
    /// enforce this invariant.
    pub fn services(&self) -> providers::Result<Vec<EnvironmentService>> {
        match self {
            OrchestratorEnvironment::Null(_) => Ok(Vec::new()),
            OrchestratorEnvironment::DockerCompose(dce) => Ok(services_from_compose_contents(
                dce.inline_compose_contents()?,
            )),
        }
    }

    /// The template variables that this environment's compose file providers depend on (if any).
    pub fn compose_template_variables(&self) -> Vec<String> {
        match self {
            OrchestratorEnvironment::Null(_) => Vec::new(),
            OrchestratorEnvironment::DockerCompose(dce) => dce
                .compose_files
                .iter()
                .flat_map(|ncfp| ncfp.required_variables())
                .collect(),
        }
    }
}

enum_impl_check!(OrchestratorEnvironment => Null, DockerCompose);

impl ValidateEnvironment for OrchestratorEnvironment {}

impl RunEnvironment for OrchestratorEnvironment {
    async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        match self {
            OrchestratorEnvironment::Null(inner) => inner.execute_setup(name, out_dir, ctx).await,
            OrchestratorEnvironment::DockerCompose(inner) => {
                inner.execute_setup(name, out_dir, ctx).await
            }
        }
    }

    async fn execute_teardown(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        match self {
            OrchestratorEnvironment::Null(inner) => {
                inner.execute_teardown(name, out_dir, ctx).await
            }
            OrchestratorEnvironment::DockerCompose(inner) => {
                inner.execute_teardown(name, out_dir, ctx).await
            }
        }
    }
}

impl RunProviders for OrchestratorEnvironment {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        match self {
            OrchestratorEnvironment::Null(inner) => inner.named_providers(),
            OrchestratorEnvironment::DockerCompose(inner) => inner.named_providers(),
        }
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        match self {
            OrchestratorEnvironment::Null(inner) => inner.inline(mode, ctx, cache),
            OrchestratorEnvironment::DockerCompose(inner) => inner.inline(mode, ctx, cache),
        }
    }
}

impl CheckArrayDuplicates for OrchestratorEnvironment {
    const BASE_PATH: &str = "environment_execution";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        match self {
            OrchestratorEnvironment::Null(inner) => inner.deduplicated_arrays(),
            OrchestratorEnvironment::DockerCompose(inner) => inner.deduplicated_arrays(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentService {
    pub name: String,
    pub image: Option<String>,
    pub replicas: ServiceReplicas,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceReplicas {
    Fixed(u32),
    Variable(String),
}

impl ServiceReplicas {
    pub fn is_fixed(&self) -> bool {
        matches!(self, Self::Fixed(_))
    }
}

#[derive(Debug, Default)]
struct PartialService {
    image: Option<String>,
    replicas: Option<ServiceReplicas>,
}

fn services_from_compose_contents(contents: Vec<&str>) -> Vec<EnvironmentService> {
    let mut merged: BTreeMap<String, PartialService> = BTreeMap::new();

    for content in contents.into_iter() {
        merge_services_from(&mut merged, content);
    }

    merged
        .into_iter()
        .map(|(name, partial)| EnvironmentService {
            name,
            image: partial.image,
            replicas: partial.replicas.unwrap_or(ServiceReplicas::Fixed(1)),
        })
        .collect()
}

fn merge_services_from(
    merged: &mut BTreeMap<String, PartialService>,
    yaml_content: &str,
) -> Option<()> {
    let value = serde_yaml::from_str::<Value>(yaml_content).ok()?;
    let services = value.get("services").and_then(|s| s.as_mapping())?;

    for (name, service) in services.into_iter() {
        let name = match name.as_str() {
            Some(name) => name,
            None => continue,
        };

        let entry = merged.entry(name.to_string()).or_default();

        if let Some(image) = service.get("image").and_then(|v| v.as_str()) {
            entry.image = Some(image.to_string());
        }
        if let Some(replicas) = replicas_from(service) {
            entry.replicas = Some(replicas);
        }
    }

    Some(())
}

fn replicas_from(service: &Value) -> Option<ServiceReplicas> {
    let raw = service
        .get("deploy")
        .and_then(|d| d.get("replicas"))
        .or_else(|| service.get("scale"))?;

    match raw {
        Value::Number(n) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(ServiceReplicas::Fixed),

        Value::String(s) => Some(match s.parse::<u32>() {
            Ok(n) => ServiceReplicas::Fixed(n),
            Err(_) => ServiceReplicas::Variable(s.clone()),
        }),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rtf_config::context::Context;
    use std::assert_matches;

    fn null_env() -> OrchestratorEnvironment {
        OrchestratorEnvironment::Null(NullEnvironment { skip: true })
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

    fn compose_env_with(providers: &[&str]) -> OrchestratorEnvironment {
        let compose_files = providers
            .iter()
            .map(|yaml| {
                serde_yaml::from_str(yaml).expect("compose file provider should deserialize")
            })
            .collect();

        OrchestratorEnvironment::DockerCompose(DockerComposeEnvironment {
            compose_files,
            ..docker_compose_env()
        })
    }

    fn services_of(providers: &[&str]) -> Vec<EnvironmentService> {
        compose_env_with(providers)
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
        use super::ServiceReplicas::{Fixed, Variable};

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
    fn services_is_empty_for_null_environment() {
        let services = null_env()
            .services()
            .expect("a null environment has no compose files to inline");

        assert!(services.is_empty());
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

        let res = compose_env_with(&[REQUIRED_COMPOSE]).services();

        assert_matches!(
            res,
            Err(providers::Error::ComposeFileNotInlined { ref name }) if name == "required-compose",
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
            compose_env_with(&[TEMPLATED_COMPOSE_PATH]).compose_template_variables(),
            vec!["compose_dir".to_string()]
        );
    }
}
