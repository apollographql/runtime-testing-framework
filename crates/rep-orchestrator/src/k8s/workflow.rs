use kube::CustomResource;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(CustomResource, Default, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "argoproj.io",
    version = "v1alpha1",
    kind = "Workflow",
    namespaced,
    status = "WorkflowStatus",
    derive = "Default"
)]
pub struct WorkflowSpec {
    #[schemars(schema_with = "any_schema")]
    pub spec: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct WorkflowStatus {
    pub phase: Option<String>, // Pending | Running | Succeeded | Failed | Error
    pub message: Option<String>,
}

fn any_schema(_: &mut SchemaGenerator) -> Schema {
    true.into()
}
