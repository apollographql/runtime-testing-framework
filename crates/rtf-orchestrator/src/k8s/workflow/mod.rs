use crate::{
    db::TestExecution,
    k8s::{CLI_BINARY, workflow::as_workflow::AsWorkflowTasks},
};
use k8s_openapi::api::core::v1::{Container, EnvVar, SecretVolumeSource, Volume, VolumeMount};
use kube::CustomResource;
use rtf_orchestrator_shared::{OtelConfig, test_plan::OrchestratorEnvironment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod as_workflow;

const CREATE_NAMESPACE: &str = "create-namespace";
const TTL_SECONDS_AFTER_FINISHED: i32 = 60; // cleanup after 1m - well clear of the 10s poll interval
const TTL_SECONDS_AFTER_FAILED: i32 = 120; // cleanup after 2m when failed for debugging

/// Mount point for the workload-cluster kubeconfig secret.
const KUBECONFIG_PATH: &str = "/kubeconfig/value";

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct WorkflowStatus {
    pub phase: Option<String>, // Pending | Running | Succeeded | Failed | Error
    pub message: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TtlStrategy {
    pub seconds_after_completion: Option<i32>,
    pub seconds_after_failure: Option<i32>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodGC {
    pub strategy: String,
}

#[derive(CustomResource, Default, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "argoproj.io",
    version = "v1alpha1",
    kind = "Workflow",
    namespaced,
    status = "WorkflowStatus",
    derive = "Default"
)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSpec {
    pub service_account_name: String,
    pub entrypoint: String,
    pub templates: Vec<TemplateDef>,
    pub volumes: Vec<Volume>,
    pub ttl_strategy: Option<TtlStrategy>,
    pub pod_g_c: Option<PodGC>,
}

/// Toolbox-related settings needed to build the tasks in a per-execution deploy workflow.
pub struct WorkflowToolboxSettings<'a> {
    pub orchestrator_url: &'a str,
    pub pull_policy: &'a str,
    pub image: &'a str,
    pub otel: &'a OtelConfig,
}

impl WorkflowSpec {
    pub fn for_execution(
        ex: &TestExecution,
        env: &OrchestratorEnvironment,
        toolbox: &WorkflowToolboxSettings<'_>,
        kubeconfig_secret_name: &str,
        exclusive_nodes: bool,
    ) -> Self {
        let execution_id = ex.uuid();
        let env_vars = ex.toolbox_env_vars(toolbox.orchestrator_url);
        let namespace = execution_id.to_string();

        let mut templates = vec![
            TemplateDef::Main(MainTemplate::new(env)),
            TemplateDef::Task(create_namespace(
                &namespace,
                toolbox.pull_policy,
                toolbox.image,
                env_vars.clone(),
            )),
        ];
        templates.extend(env.tasks(
            &namespace,
            toolbox.pull_policy,
            toolbox.image,
            toolbox.otel,
            exclusive_nodes,
            env_vars.clone(),
        ));

        Self {
            service_account_name: "argo-workflow".to_owned(),
            entrypoint: "main".to_owned(),
            templates,
            volumes: vec![Volume {
                name: "kubeconfig".into(),
                secret: Some(SecretVolumeSource {
                    secret_name: Some(kubeconfig_secret_name.into()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ttl_strategy: Some(TtlStrategy {
                seconds_after_completion: Some(TTL_SECONDS_AFTER_FINISHED),
                seconds_after_failure: Some(TTL_SECONDS_AFTER_FAILED),
            }),
            // Delete the workflow pods if the workflow succeeds
            // Retains failed pods for debugging
            pod_g_c: Some(PodGC {
                strategy: "OnWorkflowSuccess".to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[allow(clippy::large_enum_variant)]
#[serde(untagged)]
pub enum TemplateDef {
    Main(MainTemplate),
    Task(TaskTemplate),
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MainTemplate {
    pub name: String,
    pub dag: Dag,
}

impl MainTemplate {
    pub fn new(env: &OrchestratorEnvironment) -> Self {
        let mut tasks = vec![TaskSpec::new(CREATE_NAMESPACE, &[])];
        tasks.extend(env.specs(CREATE_NAMESPACE));

        Self {
            name: "main".into(),
            dag: Dag { tasks },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Dag {
    pub tasks: Vec<TaskSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TaskSpec {
    pub name: String,
    pub template: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

impl TaskSpec {
    fn new(name: &str, deps: &[&str]) -> Self {
        Self {
            name: name.into(),
            template: name.into(),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TaskTemplate {
    pub name: String,
    pub container: Container,
    pub volumes: Option<Vec<Volume>>,
}

impl TaskTemplate {
    /// Build a task container that runs `rtf-orchestrator-cli <args...>` inside the toolbox image.
    fn new(
        name: &str,
        toolbox_pull_policy: &str,
        toolbox_image: &str,
        cli_args: Vec<String>,
        volume_mounts: Vec<VolumeMount>,
        volumes: Option<Vec<Volume>>,
        env: Vec<EnvVar>,
    ) -> Self {
        Self {
            name: name.into(),
            container: Container {
                name: name.into(),
                image: Some(toolbox_image.into()),
                image_pull_policy: Some(toolbox_pull_policy.into()),
                command: Some(vec![CLI_BINARY.to_owned()]),
                args: Some(cli_args),
                volume_mounts: Some(volume_mounts),
                env: Some(env),
                ..Default::default()
            },
            volumes,
        }
    }
}

fn kubeconfig_volume_mount() -> VolumeMount {
    VolumeMount {
        name: "kubeconfig".into(),
        mount_path: "/kubeconfig".into(),
        read_only: Some(true),
        ..Default::default()
    }
}

fn create_namespace(
    namespace: &str,
    toolbox_pull_policy: &str,
    toolbox_image: &str,
    env: Vec<EnvVar>,
) -> TaskTemplate {
    TaskTemplate::new(
        CREATE_NAMESPACE,
        toolbox_pull_policy,
        toolbox_image,
        vec![
            "create-namespace".into(),
            "--namespace".into(),
            namespace.into(),
            "--kubeconfig".into(),
            KUBECONFIG_PATH.into(),
        ],
        vec![kubeconfig_volume_mount()],
        None,
        env,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::workflow::as_workflow::DEPLOY_ENVIRONMENT;
    use rtf_config::formats::{ComposeResources, DockerComposeEnvironment, NullEnvironment};

    #[test]
    fn main_template_includes_deploy_environment_for_docker_compose() {
        let env = OrchestratorEnvironment::DockerCompose(DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: vec![],
            },
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: Default::default(),
        });

        let main = MainTemplate::new(&env);

        let names: Vec<&str> = main.dag.tasks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec![CREATE_NAMESPACE, DEPLOY_ENVIRONMENT]);

        let deploy = &main.dag.tasks[1];
        assert_eq!(deploy.dependencies, vec![CREATE_NAMESPACE.to_string()]);
    }

    #[test]
    fn main_template_omits_deploy_environment_for_null() {
        let env = OrchestratorEnvironment::Null(NullEnvironment { skip: true });
        let main = MainTemplate::new(&env);

        let names: Vec<&str> = main.dag.tasks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec![CREATE_NAMESPACE]);
    }
}
