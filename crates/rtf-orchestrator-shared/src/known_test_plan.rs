use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A Test Plan registered with the orchestrator, identifiable by UUID or name for triggering and
/// for querying its historic runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownTestPlanSummary {
    pub uuid: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub org: String,
    pub repo: String,
    pub path: String,
    pub pinned_workload_cluster: Option<String>,
}

/// A page of [KnownTestPlanSummary]s matching a set of query filters, along with the total number
/// matching those filters (ignoring pagination) so that callers can page through the full result
/// set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownTestPlanListResponse {
    pub test_plans: Vec<KnownTestPlanSummary>,
    pub total: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegisterTestPlanRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub org: String,
    pub repo: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SetPinnedWorkloadClusterRequest {
    pub cluster: String,
}

/// Query parameters accepted by `GET /test-plan`. Shared between the orchestrator's `axum` `Query`
/// extractor (deserializing an inbound request) and the UI's outbound request builder
/// (serializing these as the request's query string) so the two sides can't drift apart.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnownTestPlanListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

/// Query parameters accepted by `GET /test-plan/{uuid}/runs`. Shared between the orchestrator's
/// `axum` `Query` extractor and the UI's outbound request builder, as with
/// [KnownTestPlanListParams].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnownTestPlanRunsParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiated_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_after: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_before: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}
