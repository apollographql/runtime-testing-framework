use super::format_rfc3339;
use crate::{
    links::{LinksConfig, gcp_logs, grafana},
    status,
};
use chrono::Utc;
use rtf_orchestrator_shared::{status::StatusUpdate, summary::TestExecutionSummary};
use uuid::Uuid;

/// The execution detail page: the execution's status-history timeline plus its metadata.
pub struct ExecutionDetailView {
    pub id: Uuid,
    /// The parent test run's id, used for the "back to run" link.
    pub run_id: Option<Uuid>,
    pub name: String,
    pub status_label: String,
    pub status_class: &'static str,
    pub exit_code: Option<i32>,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    /// Status updates, newest-first (as returned by the orchestrator).
    pub history: Vec<StatusEntryView>,
    /// Cloud Logging deep link scoped to this execution's workload namespace and time window.
    pub logs_url: String,
    /// Grafana deep link scoped to this execution's workload namespace and time window.
    pub grafana_url: String,
}

impl ExecutionDetailView {
    pub fn new(execution: TestExecutionSummary, links_cfg: &LinksConfig) -> Self {
        let namespace = execution.id.to_string();
        let end = execution.completed_at.unwrap_or_else(Utc::now);
        let logs_url = gcp_logs(links_cfg, &namespace, execution.started_at, end);
        let grafana_url = grafana(links_cfg, &namespace, execution.started_at, end);

        Self {
            id: execution.id,
            run_id: execution.test_run_id,
            name: execution.name,
            status_label: execution.current_status.to_string(),
            status_class: status::css_class(execution.current_status),
            exit_code: status::effective_exit_code(execution.current_status, execution.exit_code),
            started_at: format_rfc3339(execution.started_at),
            updated_at: format_rfc3339(execution.updated_at),
            completed_at: execution.completed_at.map(format_rfc3339),
            history: execution
                .status_history
                .into_iter()
                .map(StatusEntryView::from)
                .collect(),
            logs_url,
            grafana_url,
        }
    }
}

/// A single entry in an execution's status-history timeline.
pub struct StatusEntryView {
    pub status_label: String,
    pub status_class: &'static str,
    pub message: Option<String>,
    pub updated_at: String,
}

impl From<StatusUpdate> for StatusEntryView {
    fn from(update: StatusUpdate) -> Self {
        Self {
            status_label: update.status.to_string(),
            status_class: status::css_class(update.status),
            message: update.message,
            updated_at: format_rfc3339(update.updated_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        links::sample_config, orchestrator::mocks::sample_execution, templates::ExecutionTemplate,
    };
    use askama::Template;
    use chrono::TimeZone;
    use rtf_orchestrator_shared::status::Status;

    fn snapshot_execution(run_id: Option<Uuid>, ex_id: Uuid) -> TestExecutionSummary {
        let started = Utc.with_ymd_and_hms(2024, 3, 15, 9, 0, 0).unwrap();
        let completed = started + chrono::Duration::minutes(5);
        TestExecutionSummary {
            id: ex_id,
            test_run_id: run_id,
            name: "exec-alpha".to_owned(),
            current_status: Status::Successful,
            exit_code: Some(0),
            started_at: started,
            updated_at: completed,
            completed_at: Some(completed),
            status_history: vec![
                StatusUpdate {
                    status: Status::Successful,
                    message: Some("execution finished".to_owned()),
                    updated_at: completed,
                },
                StatusUpdate {
                    status: Status::Running,
                    message: None,
                    updated_at: started,
                },
            ],
        }
    }

    #[test]
    fn execution_template_renders_status_history_and_metadata() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let view = ExecutionDetailView::new(sample_execution(run_id, ex_id), &sample_config());
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        assert!(body.contains("exec-alpha"), "execution name should render");
        assert!(
            body.contains(&format!("resource.labels.namespace_name%3D%22{ex_id}%22")),
            "execution detail page should link to logs scoped to its own namespace"
        );
        assert!(
            body.contains(&format!("var-namespace={ex_id}")),
            "execution detail page should link to a Grafana dashboard scoped to its own namespace"
        );
        assert!(
            body.contains("Status history"),
            "history section should render"
        );
        assert!(
            body.contains("execution finished"),
            "history entry messages should render"
        );
        // Both history entries' statuses appear.
        assert!(body.contains("RUNNING") && body.contains("SUCCESSFUL"));
        assert!(
            body.contains(&format!("/ui/run/{run_id}")),
            "execution detail page should link back to its parent run"
        );
    }

    #[test]
    fn execution_detail_view_has_no_run_id_when_the_execution_has_no_parent_run() {
        let mut execution = sample_execution(Uuid::from_u128(1), Uuid::from_u128(2));
        execution.test_run_id = None;
        let view = ExecutionDetailView::new(execution, &sample_config());

        assert_eq!(view.run_id, None);
    }

    #[test]
    fn execution_template_snapshot_with_parent_run() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let view = ExecutionDetailView::new(snapshot_execution(Some(run_id), ex_id), &sample_config());
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        insta::assert_snapshot!(body);
    }

    #[test]
    fn execution_template_snapshot_without_parent_run() {
        let ex_id = Uuid::from_u128(2);
        let view = ExecutionDetailView::new(snapshot_execution(None, ex_id), &sample_config());
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        insta::assert_snapshot!(body);
    }
}
