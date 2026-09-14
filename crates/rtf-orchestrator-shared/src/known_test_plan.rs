use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
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
    pub allow_k8s_write: bool,
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

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UpdateKnownTestPlanRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "explicit_null"
    )]
    pub description: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "explicit_null"
    )]
    pub pinned_cluster: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_k8s_write: Option<bool>,
}

impl UpdateKnownTestPlanRequest {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Distinguish explicit null values from being omitted from the payload entirely:
///
///  field missing -> None
///  explicit null -> Some(None)
///  actual value  -> Some(Some(value))
fn explicit_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
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
