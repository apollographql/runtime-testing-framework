use crate::{
    links::{LinksConfig, gcp_logs, grafana},
    status,
    view::{StatusView, exit_code_label, format_rfc3339},
};
use chrono::{DateTime, Utc};
use humantime::format_duration;
use rtf_orchestrator_shared::{
    status::Status,
    summary::{TestExecutionSummary, TestRunSummary},
};
use std::time::Duration;
use url::form_urlencoded;
use uuid::Uuid;

/// Upper bound on how long a non-terminal run is auto-refreshed. A run that never reaches a
/// terminal state (e.g. a stuck orchestrator) would otherwise be polled forever; past this age the
/// UI stops polling and the user can refresh manually.
const MAX_POLL_AGE_SECS: i64 = 60 * 60;

pub struct RunView {
    pub id: Uuid,
    pub name: String,
    pub status: StatusView,
    pub initiated_by: String,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub elapsed: String,
    pub last_updated: String,
    pub should_poll: bool,
    pub total_executions: usize,
    pub execution_status_filter: String,
    pub status_breakdown: Vec<StatusCountView>,
    pub executions: Vec<ExecutionView>,
}

impl RunView {
    pub fn new(
        run: TestRunSummary,
        now: DateTime<Utc>,
        links_cfg: &LinksConfig,
        execution_status_filter: String,
    ) -> Self {
        let completed_at = run
            .completed_at
            .filter(|_| run.current_status.is_terminal());
        let end = completed_at.unwrap_or(now);
        let total_executions = run.executions.len();
        let status_breakdown = status_breakdown(&run.executions);

        Self {
            id: run.id,
            name: run.name,
            status: run.current_status.into(),
            initiated_by: run.initiated_by,
            started_at: format_rfc3339(run.started_at),
            updated_at: format_rfc3339(run.updated_at),
            completed_at: completed_at.map(format_rfc3339),
            elapsed: format_duration(Duration::from_secs(
                (end.timestamp() - run.started_at.timestamp()).max(0) as u64,
            ))
            .to_string(),
            last_updated: now.format("%H:%M:%S UTC").to_string(),
            should_poll: should_poll(run.current_status, run.started_at, now),
            total_executions,
            status_breakdown,
            executions: run
                .executions
                .into_iter()
                .filter(|execution| {
                    execution_status_filter.is_empty()
                        || execution.current_status.to_string() == execution_status_filter
                })
                .map(|execution| ExecutionView::new(execution, links_cfg))
                .collect(),
            execution_status_filter,
        }
    }

    pub fn poll_url(&self) -> String {
        if self.execution_status_filter.is_empty() {
            format!("/ui/run/{}", self.id)
        } else {
            let mut qs = form_urlencoded::Serializer::new(String::new());
            qs.append_pair("execution_status", &self.execution_status_filter);
            format!("/ui/run/{}?{}", self.id, qs.finish())
        }
    }

    pub fn executions_empty_message(&self) -> &'static str {
        if self.execution_status_filter.is_empty() {
            "No executions yet."
        } else {
            "No executions match this filter."
        }
    }

    pub fn is_status_selected(&self, value: &str) -> bool {
        self.execution_status_filter == value
    }
}

const STATUS_LIFECYCLE_ORDER: [Status; 8] = [
    Status::Initialising,
    Status::Resolving,
    Status::Provisioning,
    Status::EnvironmentReady,
    Status::Running,
    Status::Successful,
    Status::Failed,
    Status::Unrunnable,
];

pub struct StatusCountView {
    pub status: StatusView,
    pub count: usize,
}

fn status_breakdown(executions: &[TestExecutionSummary]) -> Vec<StatusCountView> {
    STATUS_LIFECYCLE_ORDER
        .into_iter()
        .filter_map(|candidate| {
            let count = executions
                .iter()
                .filter(|execution| execution.current_status == candidate)
                .count();

            (count > 0).then(|| StatusCountView {
                status: candidate.into(),
                count,
            })
        })
        .collect()
}

pub struct ExecutionView {
    pub id: Uuid,
    pub name: String,
    pub status: StatusView,
    pub exit_code_label: String,
    pub started_at: String,
    pub updated_at: String,
    pub logs_url: String,
    pub grafana_url: String,
}

impl ExecutionView {
    fn new(execution: TestExecutionSummary, links_cfg: &LinksConfig) -> Self {
        let namespace = execution.id.to_string();
        let window_end = execution.completed_at.unwrap_or_else(Utc::now);
        let logs_url = gcp_logs(links_cfg, &namespace, execution.started_at, window_end);
        let grafana_url = grafana(links_cfg, &namespace, execution.started_at, window_end);

        Self {
            id: execution.id,
            name: execution.name,
            status: execution.current_status.into(),
            exit_code_label: exit_code_label(status::effective_exit_code(
                execution.current_status,
                execution.exit_code,
            )),
            started_at: format_rfc3339(execution.started_at),
            updated_at: format_rfc3339(execution.updated_at),
            logs_url,
            grafana_url,
        }
    }
}

fn should_poll(status: Status, started_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    !status.is_terminal() && (now.timestamp() - started_at.timestamp()) < MAX_POLL_AGE_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        links::sample_config,
        orchestrator::mocks::{sample_execution, sample_summary},
        templates::RunTemplate,
    };
    use askama::Template;
    use chrono::{Duration, TimeZone};
    use simple_test_case::test_case;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 3, 15, 12, 30, 0).unwrap()
    }

    #[test_case(Status::Running, 0, true; "recent running run polls")]
    #[test_case(Status::Running, MAX_POLL_AGE_SECS + 1, false; "stuck running run stops polling")]
    #[test_case(Status::Successful, 0, false; "successful run does not poll")]
    #[test_case(Status::Failed, 0, false; "failed run does not poll")]
    #[test_case(Status::Unrunnable, 0, false; "unrunnable run does not poll")]
    #[test]
    fn should_poll_cases(status: Status, age_secs: i64, expected: bool) {
        let now = Utc::now();
        let started = now - Duration::seconds(age_secs);
        assert_eq!(should_poll(status, started, now), expected);
    }

    #[test_case(Status::Running, false; "hidden while the run is not terminal")]
    #[test_case(Status::Successful, true; "shown once the run is terminal")]
    #[test]
    fn completed_at_visibility(status: Status, expect_shown: bool) {
        let now = Utc::now();
        let run = TestRunSummary {
            current_status: status,
            started_at: now - Duration::seconds(5),
            // Inconsistent with a non-terminal status in the non-terminal case; should be ignored.
            completed_at: Some(now),
            ..Default::default()
        };
        let completed_at = RunView::new(run, now, &sample_config(), String::new()).completed_at;

        if expect_shown {
            assert_eq!(completed_at, Some(format_rfc3339(now)));
        } else {
            assert_eq!(completed_at, None);
        }
    }

    #[test]
    fn run_template_carries_the_poll_trigger_while_non_terminal() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Running),
            Utc::now(),
            &sample_config(),
            String::new(),
        );
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(body.contains(&format!("hx-get=\"/ui/run/{run_id}\"")));
        assert!(body.contains("hx-select=\"#run\""));
    }

    #[test]
    fn run_template_omits_the_poll_trigger_once_terminal() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Successful),
            Utc::now(),
            &sample_config(),
            String::new(),
        );
        let body = RunTemplate { run }.render().expect("template renders");

        // The manual "Refresh now" button always carries `hx-get`/`hx-select`; `hx-trigger` only
        // ever appears on the auto-poll attributes, so its absence is what proves polling stopped.
        assert!(!body.contains("hx-trigger"));
    }

    #[test]
    fn run_template_renders_run_and_execution_details() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Running),
            Utc::now(),
            &sample_config(),
            String::new(),
        );
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(body.contains("my-test-run"), "run name should render");
        assert!(body.contains("RUNNING"), "run status label should render");
        assert!(
            body.contains("someone@apollographql.com"),
            "run initiator should render"
        );
        assert!(body.contains("exec-alpha"), "execution name should render");
        assert!(
            body.contains("SUCCESSFUL"),
            "execution status label should render"
        );
    }

    #[test]
    fn execution_view_links_to_its_own_gcp_logs_and_grafana_dashboard() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let view = ExecutionView::new(sample_execution(run_id, ex_id), &sample_config());

        assert!(
            view.logs_url
                .contains(&format!("resource.labels.namespace_name%3D%22{ex_id}%22")),
            "execution should link to logs scoped to its own namespace, got: {}",
            view.logs_url
        );
        assert!(
            view.grafana_url.contains(&format!("var-namespace={ex_id}")),
            "execution should link to a Grafana dashboard scoped to its own namespace, got: {}",
            view.grafana_url
        );
    }

    fn run_with_mixed_execution_statuses(run_id: Uuid) -> TestRunSummary {
        TestRunSummary {
            id: run_id,
            name: "mixed-run".to_owned(),
            current_status: Status::Failed,
            started_at: Utc::now(),
            executions: vec![
                TestExecutionSummary {
                    id: Uuid::from_u128(10),
                    name: "exec-ok".to_owned(),
                    current_status: Status::Successful,
                    ..Default::default()
                },
                TestExecutionSummary {
                    id: Uuid::from_u128(11),
                    name: "exec-broke".to_owned(),
                    current_status: Status::Failed,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn execution_status_filter_narrows_down_the_executions_shown() {
        let run_id = Uuid::from_u128(1);
        let run = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            "FAILED".to_owned(),
        );

        assert_eq!(run.total_executions, 2, "total ignores the filter");
        assert_eq!(run.executions.len(), 1, "table rows respect the filter");
        assert_eq!(run.executions[0].name, "exec-broke");
    }

    #[test]
    fn empty_execution_status_filter_keeps_every_execution() {
        let run_id = Uuid::from_u128(1);
        let run = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            String::new(),
        );

        assert_eq!(run.total_executions, 2);
        assert_eq!(run.executions.len(), 2);
    }

    #[test]
    fn poll_url_is_bare_with_no_filter_and_carries_the_filter_when_set() {
        let run_id = Uuid::from_u128(1);
        let unfiltered = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            String::new(),
        );
        assert_eq!(unfiltered.poll_url(), format!("/ui/run/{run_id}"));

        let filtered = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            "FAILED".to_owned(),
        );
        assert_eq!(
            filtered.poll_url(),
            format!("/ui/run/{run_id}?execution_status=FAILED")
        );
    }

    #[test]
    fn executions_empty_message_distinguishes_no_matches_from_no_executions_at_all() {
        let run_id = Uuid::from_u128(1);
        let no_executions = RunView::new(
            TestRunSummary {
                id: run_id,
                started_at: Utc::now(),
                ..Default::default()
            },
            Utc::now(),
            &sample_config(),
            String::new(),
        );
        assert_eq!(
            no_executions.executions_empty_message(),
            "No executions yet."
        );

        let no_matches = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            "UNRUNNABLE".to_owned(),
        );
        assert_eq!(
            no_matches.executions_empty_message(),
            "No executions match this filter."
        );
    }

    #[test_case("", "", true; "empty filter matches the all-statuses option")]
    #[test_case("", "FAILED", false; "empty filter does not match a status option")]
    #[test_case("FAILED", "FAILED", true; "matching status is selected")]
    #[test_case("FAILED", "SUCCESSFUL", false; "non-matching status is not selected")]
    #[test]
    fn is_status_selected_cases(execution_status_filter: &str, value: &str, expected: bool) {
        let run_id = Uuid::from_u128(1);
        let run = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            execution_status_filter.to_owned(),
        );

        assert_eq!(run.is_status_selected(value), expected);
    }

    #[test]
    fn status_breakdown_counts_and_orders_by_lifecycle_ignoring_the_filter() {
        let run_id = Uuid::from_u128(1);
        let mut run = run_with_mixed_execution_statuses(run_id);
        run.executions.push(TestExecutionSummary {
            id: Uuid::from_u128(12),
            name: "exec-ok-2".to_owned(),
            current_status: Status::Successful,
            ..Default::default()
        });

        let view = RunView::new(run, Utc::now(), &sample_config(), "FAILED".to_owned());

        let labels_and_counts: Vec<(&str, usize)> = view
            .status_breakdown
            .iter()
            .map(|row| (row.status.label.as_str(), row.count))
            .collect();
        assert_eq!(
            labels_and_counts,
            vec![("SUCCESSFUL", 2), ("FAILED", 1)],
            "breakdown should count every execution in lifecycle order and ignore the active filter"
        );
    }

    #[test]
    fn run_template_renders_the_status_breakdown_table() {
        let run_id = Uuid::from_u128(1);
        let run = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            String::new(),
        );
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(
            body.contains("<h3>Status breakdown</h3>"),
            "expected a status breakdown heading, got: {body}"
        );
        assert!(
            body.contains("SUCCESSFUL") && body.contains("<td>1</td>"),
            "expected a row counting the successful execution, got: {body}"
        );
        assert!(
            !body.contains("class=\"status status--unrunnable\""),
            "statuses with no executions should not get a status pill anywhere on the page"
        );
    }

    #[test]
    fn run_template_snapshot_running_with_mixed_statuses_and_active_filter() {
        let run_id = Uuid::from_u128(1);
        let started = Utc.with_ymd_and_hms(2024, 3, 15, 12, 0, 0).unwrap();
        let run = TestRunSummary {
            id: run_id,
            name: "nightly-smoke".to_owned(),
            current_status: Status::Running,
            initiated_by: "someone@apollographql.com".to_owned(),
            started_at: started,
            updated_at: started + Duration::minutes(5),
            executions: vec![
                TestExecutionSummary {
                    id: Uuid::from_u128(10),
                    name: "exec-ok".to_owned(),
                    current_status: Status::Successful,
                    exit_code: Some(0),
                    started_at: started,
                    updated_at: started + Duration::minutes(3),
                    completed_at: Some(started + Duration::minutes(3)),
                    ..Default::default()
                },
                TestExecutionSummary {
                    id: Uuid::from_u128(11),
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
        let view = RunView::new(run, fixed_now(), &sample_config(), "FAILED".to_owned());
        let body = RunTemplate { run: view }
            .render()
            .expect("template renders");

        insta::assert_snapshot!(body);
    }

    #[test]
    fn run_template_snapshot_terminal_run_with_completed_at() {
        let run_id = Uuid::from_u128(2);
        let ex_id = Uuid::from_u128(20);
        let started = Utc.with_ymd_and_hms(2024, 3, 15, 9, 0, 0).unwrap();
        let completed = started + Duration::minutes(12);
        let run = TestRunSummary {
            id: run_id,
            name: "release-check".to_owned(),
            current_status: Status::Successful,
            initiated_by: "someone@apollographql.com".to_owned(),
            started_at: started,
            updated_at: completed,
            completed_at: Some(completed),
            executions: vec![TestExecutionSummary {
                id: ex_id,
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
        let view = RunView::new(run, fixed_now(), &sample_config(), String::new());
        let body = RunTemplate { run: view }
            .render()
            .expect("template renders");

        insta::assert_snapshot!(body);
    }
}
