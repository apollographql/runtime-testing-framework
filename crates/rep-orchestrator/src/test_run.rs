use crate::test_execution::TestExecutionSummary;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Default, Debug, Serialize)]
pub struct TestRunSummary {
    pub id: Uuid,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub executions: Vec<TestExecutionSummary>,
}

#[derive(Default, Debug, Serialize)]
pub enum RunStatus {
    #[default]
    Initializing,
}
