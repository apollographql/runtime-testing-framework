//! Types describing the workload clusters available to the orchestrator
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterSummaryResponse {
    pub max_queued_executions: usize,
    pub pools: Vec<WorkloadPoolSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadPoolSummary {
    pub name: String,
    pub clusters: Vec<WorkloadClusterSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadClusterSummary {
    pub name: String,
    pub execution: ClusterExecutionSummary,
    pub hourly_executions: Vec<HourlyCount>,
    /// `None` when this cluster's node data could not be fetched (e.g. it is unreachable).
    pub nodes: Option<NodesSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodesSummary {
    pub by_instance_type: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterExecutionSummary {
    pub max_concurrent: usize,
    pub failed_execution_ttl_secs: u64,
    pub retry_window_secs: u64,
    pub poll_interval_secs: u64,
    pub exclusive_nodes: bool,
    pub scenario_node_selector: BTreeMap<String, String>,
    pub namespace_cleanup_timeout_secs: u64,
    pub per_user: PerUserExecutionSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerUserExecutionSummary {
    pub max_concurrent_runs: usize,
    pub max_queued_runs: usize,
    pub max_queued_executions: usize,
    pub max_runs_per_hour: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HourlyCount {
    pub hour: DateTime<Utc>,
    pub count: u64,
}
