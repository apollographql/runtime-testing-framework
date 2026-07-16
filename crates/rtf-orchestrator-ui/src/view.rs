use std::time::Duration;

use crate::links::{gcp_logs, grafana};
use crate::status;
use chrono::{DateTime, Utc};
use humantime::format_duration;
use rep_orchestrator_shared::status::{Status, StatusUpdate};
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunSummary};
use uuid::Uuid;

/// Upper bound on how long a non-terminal run is auto-refreshed. A run that never reaches a
/// terminal state (e.g. a stuck orchestrator) would otherwise be polled forever; past this age the
/// UI stops polling and the user can refresh manually.
const MAX_POLL_AGE_SECS: i64 = 60 * 60;

/// The run status page content: the overall-status banner and the executions table. This whole
/// region is re-rendered on every htmx poll (via `hx-select` against the same page route), so the
/// banner, elapsed time, and "last updated" indicator all refresh together and stop together on
/// terminal.
pub struct RunView {
    pub id: Uuid,
    pub name: String,
    pub status_label: &'static str,
    pub status_class: &'static str,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    /// Wall-clock time from start to completion (or to `now` while still running).
    pub elapsed: String,
    /// When this render happened, shown as the "last updated" indicator.
    pub last_updated: String,
    /// Whether the rendered region should keep polling for updates (drives the htmx trigger).
    pub should_poll: bool,
    pub executions: Vec<ExecutionView>,
}

impl RunView {
    /// Build the run view. `now` decides whether the region keeps polling and anchors the elapsed
    /// time and "last updated" indicator.
    pub fn new(run: TestRunSummary, now: DateTime<Utc>) -> Self {
        // Trust `current_status`, not just the presence of `completed_at`, to decide whether the run
        // is actually done — a run that is still in progress should never show a completion time,
        // even if the wire data is momentarily inconsistent.
        let completed_at = run
            .completed_at
            .filter(|_| run.current_status.is_terminal());
        let end = completed_at.unwrap_or(now);

        Self {
            id: run.id,
            name: run.name,
            status_label: status::label(run.current_status),
            status_class: status::css_class(run.current_status),
            started_at: run.started_at.to_rfc3339(),
            updated_at: run.updated_at.to_rfc3339(),
            completed_at: completed_at.map(|ts| ts.to_rfc3339()),
            elapsed: format_duration(Duration::from_secs(
                (end.timestamp() - run.started_at.timestamp()).max(0) as u64,
            ))
            .to_string(),
            last_updated: now.format("%H:%M:%S UTC").to_string(),
            should_poll: should_poll(run.current_status, run.started_at, now),
            executions: run
                .executions
                .into_iter()
                .map(ExecutionView::from)
                .collect(),
        }
    }
}

/// The execution view within a test run summary
pub struct ExecutionView {
    pub id: Uuid,
    pub name: String,
    pub status_label: &'static str,
    pub status_class: &'static str,
    pub exit_code: Option<i32>,
    pub started_at: String,
    pub updated_at: String,
    /// Cloud Logging deep link scoped to this execution's workload namespace and time window.
    pub logs_url: String,
    /// Grafana deep link scoped to this execution's workload namespace and time window.
    pub grafana_url: String,
}

impl From<TestExecutionSummary> for ExecutionView {
    fn from(execution: TestExecutionSummary) -> Self {
        let namespace = execution.id.to_string();
        let window_end = execution.completed_at.unwrap_or_else(Utc::now);
        let logs_url = gcp_logs(&namespace, execution.started_at, window_end);
        let grafana_url = grafana(&namespace, execution.started_at, window_end);

        Self {
            id: execution.id,
            name: execution.name,
            status_label: status::label(execution.current_status),
            status_class: status::css_class(execution.current_status),
            exit_code: execution.exit_code,
            started_at: execution.started_at.to_rfc3339(),
            updated_at: execution.updated_at.to_rfc3339(),
            logs_url,
            grafana_url,
        }
    }
}

/// The execution detail page: the execution's status-history timeline plus its metadata.
pub struct ExecutionDetailView {
    /// The parent test run's id, used for the "back to run" link.
    pub run_id: Option<Uuid>,
    pub name: String,
    pub status_label: &'static str,
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
    pub fn new(execution: TestExecutionSummary) -> Self {
        let namespace = execution.id.to_string();
        let end = execution.completed_at.unwrap_or_else(Utc::now);
        let logs_url = gcp_logs(&namespace, execution.started_at, end);
        let grafana_url = grafana(&namespace, execution.started_at, end);

        Self {
            run_id: execution.test_run_id,
            name: execution.name,
            status_label: status::label(execution.current_status),
            status_class: status::css_class(execution.current_status),
            exit_code: execution.exit_code,
            started_at: execution.started_at.to_rfc3339(),
            updated_at: execution.updated_at.to_rfc3339(),
            completed_at: execution.completed_at.map(|ts| ts.to_rfc3339()),
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
    pub status_label: &'static str,
    pub status_class: &'static str,
    pub message: Option<String>,
    pub updated_at: String,
}

impl From<StatusUpdate> for StatusEntryView {
    fn from(update: StatusUpdate) -> Self {
        Self {
            status_label: status::label(update.status),
            status_class: status::css_class(update.status),
            message: update.message,
            updated_at: update.updated_at.to_rfc3339(),
        }
    }
}

/// Whether a run should keep being polled: only while it is non-terminal and has not been running
/// longer than [`MAX_POLL_AGE_SECS`] (the stuck-run guard).
fn should_poll(status: Status, started_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    !status.is_terminal() && (now.timestamp() - started_at.timestamp()) < MAX_POLL_AGE_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        orchestrator::mocks::{sample_execution, sample_summary},
        templates::{ExecutionTemplate, RunTemplate},
    };
    use askama::Template;
    use chrono::Duration;

    #[test]
    fn terminal_runs_do_not_poll() {
        let now = Utc::now();
        for status in [Status::Successful, Status::Failed, Status::Unrunnable] {
            assert!(!should_poll(status, now, now), "{status:?} should not poll");
        }
    }

    #[test]
    fn recent_non_terminal_runs_poll() {
        let now = Utc::now();
        assert!(should_poll(Status::Running, now, now));
    }

    #[test]
    fn stuck_non_terminal_runs_stop_polling() {
        let now = Utc::now();
        let started = now - Duration::seconds(MAX_POLL_AGE_SECS + 1);
        assert!(!should_poll(Status::Running, started, now));
    }

    #[test]
    fn completed_at_is_hidden_while_the_run_is_not_terminal() {
        let now = Utc::now();
        let run = TestRunSummary {
            current_status: Status::Running,
            started_at: now - Duration::seconds(5),
            completed_at: Some(now), // inconsistent with a non-terminal status; should be ignored
            ..Default::default()
        };

        assert_eq!(RunView::new(run, now).completed_at, None);
    }

    #[test]
    fn completed_at_is_shown_once_the_run_is_terminal() {
        let now = Utc::now();
        let run = TestRunSummary {
            current_status: Status::Successful,
            started_at: now - Duration::seconds(5),
            completed_at: Some(now),
            ..Default::default()
        };

        assert_eq!(RunView::new(run, now).completed_at, Some(now.to_rfc3339()));
    }

    #[test]
    fn run_template_carries_the_poll_trigger_while_non_terminal() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(sample_summary(run_id, ex_id, Status::Running), Utc::now());
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
        let run = RunView::new(sample_summary(run_id, ex_id, Status::Running), Utc::now());
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(body.contains("my-test-run"), "run name should render");
        assert!(body.contains("RUNNING"), "run status label should render");
        assert!(body.contains("exec-alpha"), "execution name should render");
        assert!(
            body.contains("SUCCESSFUL"),
            "execution status label should render"
        );
    }

    #[test]
    fn run_template_links_each_execution_to_its_gcp_logs_and_grafana_dashboard() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(sample_summary(run_id, ex_id, Status::Running), Utc::now());
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(
            body.contains(&format!("resource.labels.namespace_name%3D%22{ex_id}%22")),
            "execution row should link to logs scoped to its own namespace"
        );
        assert!(
            body.contains(&format!("var-namespace={ex_id}")),
            "execution row should link to a Grafana dashboard scoped to its own namespace"
        );
    }

    #[test]
    fn run_template_shows_polish_indicators() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(sample_summary(run_id, ex_id, Status::Running), Utc::now());
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(
            body.contains("last updated"),
            "last-updated indicator present"
        );
        assert!(
            body.contains("Refresh now"),
            "manual refresh control present"
        );
        assert!(body.contains("Elapsed"), "elapsed time shown on the banner");
    }

    #[test]
    fn execution_template_renders_status_history_and_metadata() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let view = ExecutionDetailView::new(sample_execution(run_id, ex_id));
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
    fn execution_template_omits_the_back_link_when_the_run_id_is_unknown() {
        let mut execution = sample_execution(Uuid::from_u128(1), Uuid::from_u128(2));
        execution.test_run_id = None;
        let view = ExecutionDetailView::new(execution);
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        assert!(!body.contains("Back to run"));
    }
}
