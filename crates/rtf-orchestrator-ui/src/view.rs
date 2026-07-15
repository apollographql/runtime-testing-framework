use crate::status;
use chrono::{DateTime, Utc};
use rep_orchestrator_shared::status::Status;
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
            elapsed: format_elapsed(end.timestamp() - run.started_at.timestamp()),
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

pub struct ExecutionView {
    pub id: Uuid,
    pub name: String,
    pub status_label: &'static str,
    pub status_class: &'static str,
    pub exit_code: Option<i32>,
    pub started_at: String,
    pub updated_at: String,
}

impl From<TestExecutionSummary> for ExecutionView {
    fn from(execution: TestExecutionSummary) -> Self {
        Self {
            id: execution.id,
            name: execution.name,
            status_label: status::label(execution.current_status),
            status_class: status::css_class(execution.current_status),
            exit_code: execution.exit_code,
            started_at: execution.started_at.to_rfc3339(),
            updated_at: execution.updated_at.to_rfc3339(),
        }
    }
}

/// Whether a run should keep being polled: only while it is non-terminal and has not been running
/// longer than [`MAX_POLL_AGE_SECS`] (the stuck-run guard).
fn should_poll(status: Status, started_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    !status.is_terminal() && (now.timestamp() - started_at.timestamp()) < MAX_POLL_AGE_SECS
}

/// Format a duration in seconds as a compact `1h 2m 3s`, dropping leading zero units.
fn format_elapsed(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (hours, minutes, secs) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}h {minutes}m {secs}s")
    } else if minutes > 0 {
        format!("{minutes}m {secs}s")
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn format_elapsed_drops_leading_zero_units() {
        assert_eq!(format_elapsed(5), "5s");
        assert_eq!(format_elapsed(65), "1m 5s");
        assert_eq!(format_elapsed(3_665), "1h 1m 5s");
        assert_eq!(format_elapsed(-10), "0s");
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
}
