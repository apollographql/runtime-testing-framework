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
