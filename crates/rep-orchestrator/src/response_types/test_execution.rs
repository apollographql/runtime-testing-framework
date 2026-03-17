use crate::db::status::{Status, StatusUpdate};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Default, Debug, Serialize, Deserialize)]
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
