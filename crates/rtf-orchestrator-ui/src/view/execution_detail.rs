use crate::{
    links::{LinksConfig, gcp_logs, grafana},
    status::effective_exit_code,
    view::{StatusView, exit_code_label, format_rfc3339},
};
use chrono::Utc;
use rtf_orchestrator_shared::{status::StatusUpdate, summary::TestExecutionSummary};
use uuid::Uuid;

#[derive(Debug)]
pub struct ExecutionDetailView {
    pub id: Uuid,
    pub run_id: Uuid,
    pub test_plan_id: Option<Uuid>,
    pub name: String,
    pub status: StatusView,
    pub exit_code_label: String,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub history: Vec<StatusEntryView>,
    pub logs_url: String,
    pub grafana_url: String,
}

impl ExecutionDetailView {
    pub fn new(ex: TestExecutionSummary, links_cfg: &LinksConfig) -> Self {
        let namespace = ex.id.to_string();
        let end = ex.completed_at.unwrap_or_else(Utc::now);
        let cluster = ex
            .cluster
            .as_deref()
            .expect("an execution fetched standalone always carries its cluster");
        let logs_url = gcp_logs(links_cfg, cluster, &namespace, ex.started_at, end);
        let grafana_url = grafana(links_cfg, &namespace, ex.started_at, end);

        Self {
            id: ex.id,
            run_id: ex
                .test_run_id
                .expect("an execution fetched standalone always carries its parent run's id"),
            test_plan_id: ex.test_plan_id,
            name: ex.name,
            status: ex.current_status.into(),
            exit_code_label: exit_code_label(effective_exit_code(ex.current_status, ex.exit_code)),
            started_at: format_rfc3339(ex.started_at),
            updated_at: format_rfc3339(ex.updated_at),
            completed_at: ex.completed_at.map(format_rfc3339),
            history: ex
                .status_history
                .into_iter()
                .map(StatusEntryView::from)
                .collect(),
            logs_url,
            grafana_url,
        }
    }
}

#[derive(Debug)]
pub struct StatusEntryView {
    pub status: StatusView,
    pub message: Option<String>,
    pub updated_at: String,
}

impl From<StatusUpdate> for StatusEntryView {
    fn from(update: StatusUpdate) -> Self {
        Self {
            status: update.status.into(),
            message: update.message,
            updated_at: format_rfc3339(update.updated_at),
        }
    }
}
