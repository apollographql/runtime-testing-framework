use crate::{
    db::{Status, StatusUpdate},
    response_types::TestExecutionSummary,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct TestRunSummary {
    pub id: Uuid,
    pub name: String,
    pub current_status: Status,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status_history: Vec<StatusUpdate>,
    pub executions: Vec<TestExecutionSummary>,
}
