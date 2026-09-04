use crate::{
    links::{LinksConfig, gcp_logs, grafana},
    status::effective_exit_code,
    view::{StatusView, exit_code_label, format_rfc3339},
};
use chrono::Utc;
use rtf_orchestrator_shared::{status::StatusUpdate, summary::TestExecutionSummary};
use uuid::Uuid;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        links::sample_config, orchestrator::mocks::sample_execution, templates::ExecutionTemplate,
    };
    use askama::Template;
    use chrono::TimeZone;
    use rtf_orchestrator_shared::status::Status;

    fn snapshot_execution(
        test_run_id: Uuid,
        test_plan_id: Option<Uuid>,
        id: Uuid,
    ) -> TestExecutionSummary {
        let started = Utc.with_ymd_and_hms(2024, 3, 15, 9, 0, 0).unwrap();
        let completed = started + chrono::Duration::minutes(5);

        TestExecutionSummary {
            id,
            test_run_id: Some(test_run_id),
            test_plan_id,
            cluster: Some("alpha".to_owned()),
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
        assert!(body.contains("RUNNING") && body.contains("SUCCESSFUL"));
        assert!(
            body.contains(&format!("/ui/run/{run_id}")),
            "execution detail page should link back to its parent run"
        );
    }

    #[test]
    fn execution_template_links_back_to_the_test_plan_when_known() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let test_plan_id = Uuid::from_u128(3);
        let mut execution = sample_execution(run_id, ex_id);
        execution.test_plan_id = Some(test_plan_id);
        let view = ExecutionDetailView::new(execution, &sample_config());
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        assert!(
            body.contains(&format!("/ui/test-plan/{test_plan_id}")),
            "execution detail page should link back to its known test plan"
        );
    }

    #[test]
    fn execution_template_snapshot_with_test_plan() {
        let run_id = Uuid::from_u128(1);
        let plan_id = Uuid::from_u128(2);
        let ex_id = Uuid::from_u128(3);
        let view = ExecutionDetailView::new(
            snapshot_execution(run_id, Some(plan_id), ex_id),
            &sample_config(),
        );
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        insta::assert_snapshot!(body);
    }

    #[test]
    fn execution_template_snapshot_without_test_plan() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let view =
            ExecutionDetailView::new(snapshot_execution(run_id, None, ex_id), &sample_config());
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        insta::assert_snapshot!(body);
    }
}
