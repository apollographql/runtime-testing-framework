use k8s_openapi::api::core::v1::{Container, Volume};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[allow(clippy::large_enum_variant)]
#[serde(untagged)]
pub enum TemplateDef {
    Main(MainTemplate),
    Task(TaskTemplate),
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MainTemplate {
    name: String,
    dag: Dag,
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

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TaskTemplate {
    name: String,
    container: Container,
    volumes: Option<Vec<Volume>>,
}
