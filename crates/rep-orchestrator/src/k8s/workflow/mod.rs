use crate::{
    db::TestExecution,
    k8s::workflow::as_workflow::AsWorkflowTasks,
    k8s::{CLI_BINARY, TOOLBOX_IMAGE},
};
use k8s_openapi::api::core::v1::{Container, EnvVar, SecretVolumeSource, Volume, VolumeMount};
use kube::CustomResource;
use rtf_orchestrator_shared::{OtelConfig, test_plan::RepEnvironment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod as_workflow;

const CREATE_NAMESPACE: &str = "create-namespace";
const CREATE_SERVICE_ACCOUNT: &str = "create-service-account";
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

impl WorkflowSpec {
    pub fn for_execution(
        ex: &TestExecution,
        env: &RepEnvironment,
        orchestrator_url: &str,
        toolbox_pull_policy: &str,
        otel: &OtelConfig,
        kubeconfig_secret_name: &str,
    ) -> Self {
        let execution_id = ex.uuid();
        let env_vars = ex.toolbox_env_vars(orchestrator_url);
        let namespace = execution_id.to_string();

        let mut templates = vec![
            TemplateDef::Main(MainTemplate::new(env)),
            TemplateDef::Task(create_namespace(
                &namespace,
                toolbox_pull_policy,
                env_vars.clone(),
            )),
            TemplateDef::Task(create_service_account(
                &namespace,
                toolbox_pull_policy,
                env_vars.clone(),
            )),
        ];
        templates.extend(env.tasks(&namespace, toolbox_pull_policy, otel, env_vars.clone()));

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
    pub fn new(env: &RepEnvironment) -> Self {
        let mut tasks = vec![
            TaskSpec::new(CREATE_NAMESPACE, &[]),
            TaskSpec::new(CREATE_SERVICE_ACCOUNT, &[CREATE_NAMESPACE]),
        ];
        tasks.extend(env.specs(CREATE_SERVICE_ACCOUNT));

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
    /// Build a task container that runs `rep-orchestrator-cli <args...>` inside the toolbox image.
    fn new(
        name: &str,
        toolbox_pull_policy: &str,
        cli_args: Vec<String>,
        volume_mounts: Vec<VolumeMount>,
        volumes: Option<Vec<Volume>>,
        env: Vec<EnvVar>,
    ) -> Self {
        Self {
            name: name.into(),
            container: Container {
                name: name.into(),
                image: Some(TOOLBOX_IMAGE.into()),
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

fn create_namespace(namespace: &str, toolbox_pull_policy: &str, env: Vec<EnvVar>) -> TaskTemplate {
    TaskTemplate::new(
        CREATE_NAMESPACE,
        toolbox_pull_policy,
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

fn create_service_account(
    namespace: &str,
    toolbox_pull_policy: &str,
    env: Vec<EnvVar>,
) -> TaskTemplate {
    TaskTemplate::new(
        CREATE_SERVICE_ACCOUNT,
        toolbox_pull_policy,
        vec![
            "create-service-account".into(),
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
    use rtf_config::formats::{DockerComposeEnvironment, NullEnvironment};

    #[test]
    fn main_template_includes_deploy_environment_for_docker_compose() {
        let env = RepEnvironment::DockerCompose(DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: Default::default(),
        });

        let main = MainTemplate::new(&env);

        let names: Vec<&str> = main.dag.tasks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec![CREATE_NAMESPACE, CREATE_SERVICE_ACCOUNT, DEPLOY_ENVIRONMENT]
        );

        let deploy = &main.dag.tasks[2];
        assert_eq!(
            deploy.dependencies,
            vec![CREATE_SERVICE_ACCOUNT.to_string()]
        );
    }

    #[test]
    fn main_template_omits_deploy_environment_for_null() {
        let env = RepEnvironment::Null(NullEnvironment { skip: true });
        let main = MainTemplate::new(&env);

        let names: Vec<&str> = main.dag.tasks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec![CREATE_NAMESPACE, CREATE_SERVICE_ACCOUNT]);
    }
}
