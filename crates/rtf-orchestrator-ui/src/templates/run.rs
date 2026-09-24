use crate::view::RunView;
use askama::Template;

#[derive(Debug, Template)]
#[template(path = "run.html", blocks = ["back_links", "run"])]
pub struct RunTemplate {
    pub run: RunView,
}

#[derive(Debug, Template)]
#[template(path = "run_not_found.html")]
pub struct RunNotFoundTemplate {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{links::sample_config, render_fragment};
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use rtf_orchestrator_shared::{
        status::Status,
        summary::{TestExecutionSummary, TestRunSummary},
    };
    use serde_json::json;
    use uuid::Uuid;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 3, 15, 12, 30, 0).unwrap()
    }

    /// Has a test plan, trigger variables and an active execution filter.
    fn running_run() -> RunTemplate {
        let started = Utc.with_ymd_and_hms(2024, 3, 15, 12, 0, 0).unwrap();

        let run = TestRunSummary {
            id: Uuid::from_u128(1),
            test_plan_id: Some(Uuid::from_u128(2)),
            name: "nightly-smoke".to_owned(),
            cluster: "alpha".to_owned(),
            trigger_variables: Some(json!({"foo": "bar", "baz": [1, 2, 3]})),
            current_status: Status::Running,
            initiated_by: "someone@apollographql.com".to_owned(),
            started_at: started,
            updated_at: started + Duration::minutes(5),
            executions: vec![
                TestExecutionSummary {
                    id: Uuid::from_u128(3),
                    name: "exec-ok".to_owned(),
                    current_status: Status::Successful,
                    exit_code: Some(0),
                    started_at: started,
                    updated_at: started + Duration::minutes(3),
                    completed_at: Some(started + Duration::minutes(3)),
                    ..Default::default()
                },
                TestExecutionSummary {
                    id: Uuid::from_u128(4),
                    name: "exec-broke".to_owned(),
                    current_status: Status::Failed,
                    exit_code: Some(1),
                    started_at: started,
                    updated_at: started + Duration::minutes(4),
                    completed_at: Some(started + Duration::minutes(4)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        RunTemplate {
            run: RunView::new(run, fixed_now(), &sample_config(), "FAILED".to_owned()),
        }
    }

    fn terminal_run_without_test_plan() -> RunTemplate {
        let started = Utc.with_ymd_and_hms(2024, 3, 15, 9, 0, 0).unwrap();
        let completed = started + Duration::minutes(12);

        let run = TestRunSummary {
            id: Uuid::from_u128(1),
            test_plan_id: None,
            name: "release-check".to_owned(),
            cluster: "alpha".to_owned(),
            current_status: Status::Successful,
            initiated_by: "someone@apollographql.com".to_owned(),
            started_at: started,
            updated_at: completed,
            completed_at: Some(completed),
            executions: vec![TestExecutionSummary {
                id: Uuid::from_u128(3),
                name: "exec-alpha".to_owned(),
                current_status: Status::Successful,
                exit_code: Some(0),
                started_at: started,
                updated_at: completed,
                completed_at: Some(completed),
                ..Default::default()
            }],
            ..Default::default()
        };

        RunTemplate {
            run: RunView::new(run, fixed_now(), &sample_config(), String::new()),
        }
    }

    #[test]
    fn back_links_snapshot_with_test_plan() {
        insta::assert_snapshot!(render_fragment!(running_run().as_back_links()));
    }

    #[test]
    fn back_links_snapshot_without_test_plan() {
        insta::assert_snapshot!(render_fragment!(
            terminal_run_without_test_plan().as_back_links()
        ));
    }

    #[test]
    fn run_snapshot_running_with_mixed_statuses_and_active_filter() {
        insta::assert_snapshot!(render_fragment!(running_run().as_run()));
    }

    #[test]
    fn run_snapshot_terminal_without_test_plan() {
        insta::assert_snapshot!(render_fragment!(terminal_run_without_test_plan().as_run()));
    }
}
