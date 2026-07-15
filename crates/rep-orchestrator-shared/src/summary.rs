use crate::status::{Status, StatusUpdate};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRunSummary {
    pub id: Uuid,
    pub name: String,
    pub current_status: Status,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status_history: Vec<StatusUpdate>,
    pub executions: Vec<TestExecutionSummary>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestExecutionSummary {
    pub id: Uuid,
    pub name: String,
    pub current_status: Status,
    pub exit_code: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status_history: Vec<StatusUpdate>,
}
