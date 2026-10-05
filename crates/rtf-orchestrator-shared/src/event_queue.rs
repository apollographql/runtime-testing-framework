//! Types describing the current state of the orchestrator event queue
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQueueSnapshot {
    pub summary: SnapshotSummary,
    pub pools: BTreeMap<String, PoolQueueState>,
    pub clusters: BTreeMap<String, ClusterQueueState>,
    pub cached_run_payloads: Vec<Uuid>,
    pub active_run_executions: BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub resolved_execution_cache: Vec<Uuid>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    pub running: usize,
    pub queued: usize,
    pub pending_non_provisions: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolQueueState {
    pub pending_provisions: Vec<PendingProvisionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingProvisionSummary {
    pub execution_id: Uuid,
    pub run_id: Uuid,
    pub requires_dedicated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterQueueState {
    pub running_executions: Vec<Uuid>,
    pub pending_non_provisions: Vec<EventSummary>,
    pub claim: Option<ClusterClaimSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterClaimSummary {
    Reserved {
        run_id: Uuid,
        initiated_by: Option<String>,
        executions_to_wait_for: usize,
    },
    Acquiring {
        run_id: Uuid,
        initiated_by: Option<String>,
    },
    Owned {
        run_id: Uuid,
        initiated_by: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSummary {
    pub execution_id: Uuid,
    /// Informational description of the pending event
    pub event: String,
}
