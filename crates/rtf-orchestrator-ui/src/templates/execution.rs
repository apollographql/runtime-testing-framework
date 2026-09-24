use crate::view::ExecutionDetailView;
use askama::Template;

/// The execution detail page: status-history timeline and metadata.
#[derive(Debug, Template)]
#[template(path = "execution.html", blocks = ["back_links", "banner", "history"])]
pub struct ExecutionTemplate {
    pub execution: ExecutionDetailView,
}

/// Shown when no execution with the requested id exists.
#[derive(Debug, Template)]
#[template(path = "execution_not_found.html")]
pub struct ExecutionNotFoundTemplate {
    pub execution_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{links::sample_config, render_fragment};
    use chrono::{Duration, TimeZone, Utc};
    use rtf_orchestrator_shared::{
        status::{Status, StatusUpdate},
        summary::TestExecutionSummary,
    };
    use uuid::Uuid;

    fn execution(test_plan_id: Option<Uuid>) -> ExecutionTemplate {
        let started = Utc.with_ymd_and_hms(2024, 3, 15, 9, 0, 0).unwrap();
        let completed = started + Duration::minutes(5);

        let execution = TestExecutionSummary {
            id: Uuid::from_u128(3),
            test_run_id: Some(Uuid::from_u128(1)),
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
        };

        ExecutionTemplate {
            execution: ExecutionDetailView::new(execution, &sample_config()),
        }
    }

    #[test]
    fn back_links_snapshot_with_test_plan() {
        insta::assert_snapshot!(render_fragment!(
            execution(Some(Uuid::from_u128(2))).as_back_links()
        ));
    }

    #[test]
    fn back_links_snapshot_without_test_plan() {
        insta::assert_snapshot!(render_fragment!(execution(None).as_back_links()));
    }

    #[test]
    fn banner_snapshot() {
        insta::assert_snapshot!(render_fragment!(execution(None).as_banner()));
    }

    #[test]
    fn history_snapshot() {
        insta::assert_snapshot!(render_fragment!(execution(None).as_history()));
    }
}
