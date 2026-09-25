//! Types describing the current state of the orchestrator event queue
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQueueSnapshot {
    pub summary: SnapshotSummary,
    /// Queue state for each configured workload cluster, keyed by cluster name
    pub clusters: BTreeMap<String, ClusterQueueState>,
    pub cached_run_payloads: Vec<Uuid>,
    pub active_run_executions: HashMap<Uuid, HashSet<Uuid>>,
    pub resolved_execution_cache: Vec<Uuid>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    pub running: usize,
    pub queued: usize,
    pub pending_provisions: usize,
    pub pending_non_provisions: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterQueueState {
    pub running_executions: Vec<Uuid>,
    pub pending_provisions: Vec<EventSummary>,
    pub pending_non_provisions: Vec<EventSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSummary {
    pub execution_id: Uuid,
    /// Informational description of the pending event
    pub event: String,
}
