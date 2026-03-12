use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Default, Debug, Serialize)]
pub struct TestExecutionSummary {
    pub id: Uuid,
    pub name: String,
    pub status: ExecutionStatus,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Default, Debug, Serialize)]
pub enum ExecutionStatus {
    #[default]
    Initializing,
}
