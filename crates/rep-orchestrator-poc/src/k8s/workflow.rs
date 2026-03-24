use crate::k8s::TOOLBOX_IMAGE;
use k8s_openapi::api::core::v1::{
    ConfigMapVolumeSource, Container, KeyToPath, SecretVolumeSource, Volume, VolumeMount,
};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn workflow_name(execution_id: &Uuid) -> String {
    format!("provision-env-{execution_id}")
}

pub fn env_configmap_name(execution_id: &Uuid) -> String {
    format!("environment-config-{execution_id}")
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct WorkflowStatus {
    pub phase: Option<String>, // Pending | Running | Succeeded | Failed | Error
    pub message: Option<String>,
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
    pub on_exit: String,
    pub templates: Vec<TemplateDef>,
    pub volumes: Vec<Volume>,
}

impl WorkflowSpec {
    pub fn for_execution_id(execution_id: &Uuid) -> Self {
        let configmap_name = env_configmap_name(execution_id);
        let namespace = execution_id.to_string();

        Self {
            service_account_name: "argo-workflow".to_owned(),
            entrypoint: "main".to_owned(),
            on_exit: "cleanup".to_owned(),
            templates: vec![
                TemplateDef::Main(MainTemplate::new()),
                TemplateDef::Task(create_namespace(&namespace)),
                TemplateDef::Task(create_pull_secret(&namespace)),
                TemplateDef::Task(deploy_environment(&configmap_name, &namespace)),
                TemplateDef::Task(cleanup(&configmap_name)),
            ],
            volumes: vec![Volume {
                name: "kubeconfig".into(),
                secret: Some(SecretVolumeSource {
                    secret_name: Some("workload-kubeconfig".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum TemplateDef {
    Main(MainTemplate),
    Task(TaskTemplate),
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MainTemplate {
    name: String,
    dag: Dag,
}

impl MainTemplate {
    pub fn new() -> Self {
        Self {
            name: "main".into(),
            dag: Dag {
                tasks: vec![
                    TaskSpec::new("create-namespace", &[]),
                    TaskSpec::new("create-pull-secret", &["create-namespace"]),
                    TaskSpec::new("deploy-environment", &["create-pull-secret"]),
                ],
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Dag {
    tasks: Vec<TaskSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TaskSpec {
    name: String,
    template: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<String>,
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
    name: String,
    container: Container,
    volumes: Option<Vec<Volume>>,
}

impl TaskTemplate {
    fn new(
        name: &str,
        arg: String,
        volume_mounts: Vec<VolumeMount>,
        volumes: Option<Vec<Volume>>,
    ) -> Self {
        Self {
            name: name.into(),
            container: Container {
                name: name.into(),
                image: Some(TOOLBOX_IMAGE.into()),
                command: Some(vec!["/bin/sh".to_owned(), "-c".to_owned()]),
                args: Some(vec![arg]),
                volume_mounts: Some(volume_mounts),
                ..Default::default()
            },
            volumes,
        }
    }
}

const CREATE_NAMESPACE_SCRIPT: &str = r#"
rep-orchestrator-cli create-namespace \
    --namespace __NAMESPACE__ \
    --kubeconfig /kubeconfig/value
"#;

fn create_namespace(namespace: &str) -> TaskTemplate {
    TaskTemplate::new(
        "create-namespace",
        CREATE_NAMESPACE_SCRIPT.replace("__NAMESPACE__", namespace),
        vec![VolumeMount {
            name: "kubeconfig".into(),
            mount_path: "/kubeconfig".into(),
            read_only: Some(true),
            ..Default::default()
        }],
        None,
    )
}

const CREATE_PULL_SECRET_SCRIPT: &str = r#"
rep-orchestrator-cli create-pull-secret \
    --namespace __NAMESPACE__ \
    --kubeconfig /kubeconfig/value
    --docker-config /gcr-secret/config.json
"#;

fn create_pull_secret(namespace: &str) -> TaskTemplate {
    TaskTemplate::new(
        "create-pull-secret",
        CREATE_PULL_SECRET_SCRIPT.replace("__NAMESPACE__", namespace),
        vec![
            VolumeMount {
                name: "kubeconfig".into(),
                mount_path: "/kubeconfig".into(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: "gcr-secret".into(),
                mount_path: "/gcr-secret".into(),
                read_only: Some(true),
                ..Default::default()
            },
        ],
        Some(vec![Volume {
            name: "gcr-secret".into(),
            secret: Some(SecretVolumeSource {
                secret_name: Some("gcr-secret".into()),
                items: Some(vec![KeyToPath {
                    key: ".dockerconfigjson".into(),
                    path: "config.json".into(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    )
}

const DEPLOY_ENV_SCRIPT: &str = r#"
rep-orchestrator-cli deploy-environment \
    --namespace __NAMESPACE__ \
    --kubeconfig /kubeconfig/value
    --environment /environment/environment.yaml
"#;

fn deploy_environment(configmap_name: &str, namespace: &str) -> TaskTemplate {
    TaskTemplate::new(
        "deploy-environment",
        DEPLOY_ENV_SCRIPT.replace("__NAMESPACE__", namespace),
        vec![
            VolumeMount {
                name: "kubeconfig".into(),
                mount_path: "/kubeconfig".into(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: "environment".into(),
                mount_path: "/environment".into(),
                read_only: Some(true),
                ..Default::default()
            },
        ],
        Some(vec![Volume {
            name: "environment".to_owned(),
            config_map: Some(ConfigMapVolumeSource {
                name: configmap_name.to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    )
}

const CLEANUP_SCRIPT: &str = r#"
rep-orchestrator-cli cleanup --configmap __CONFIGMAP__ --namespace cluster-api
"#;

fn cleanup(configmap_name: &str) -> TaskTemplate {
    TaskTemplate::new(
        "cleanup",
        CLEANUP_SCRIPT.replace("__CONFIGMAP__", configmap_name),
        vec![],
        None,
    )
}
