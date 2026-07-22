use crate::links::{LinksConfig, gcp_logs, grafana};
use crate::status;
use chrono::{DateTime, Utc};
use humantime::format_duration;
use rep_orchestrator_shared::status::{Status, StatusUpdate};
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunListResponse, TestRunSummary};
use std::time::Duration;
use url::form_urlencoded;
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
    pub status_label: String,
    pub status_class: &'static str,
    pub initiated_by: String,
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
    /// time and "last updated" indicator. `links_cfg` supplies the GCP/Grafana deep-link
    /// configuration for each execution row.
    pub fn new(run: TestRunSummary, now: DateTime<Utc>, links_cfg: &LinksConfig) -> Self {
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
            status_label: run.current_status.to_string(),
            status_class: status::css_class(run.current_status),
            initiated_by: run.initiated_by,
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
                .map(|execution| ExecutionView::new(execution, links_cfg))
                .collect(),
        }
    }
}

/// The execution view within a test run summary
pub struct ExecutionView {
    pub id: Uuid,
    pub name: String,
    pub status_label: String,
    pub status_class: &'static str,
    pub exit_code: Option<i32>,
    pub started_at: String,
    pub updated_at: String,
    /// Cloud Logging deep link scoped to this execution's workload namespace and time window.
    pub logs_url: String,
    /// Grafana deep link scoped to this execution's workload namespace and time window.
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
            status_label: execution.current_status.to_string(),
            status_class: status::css_class(execution.current_status),
            exit_code: status::effective_exit_code(execution.current_status, execution.exit_code),
            started_at: execution.started_at.to_rfc3339(),
            updated_at: execution.updated_at.to_rfc3339(),
            logs_url,
            grafana_url,
        }
    }
}

/// One row in the recent-runs table on the home page.
pub struct RunListRowView {
    pub id: Uuid,
    pub name: String,
    pub status_label: String,
    pub status_class: &'static str,
    pub initiated_by: String,
    pub started_at: String,
}

impl From<TestRunSummary> for RunListRowView {
    fn from(run: TestRunSummary) -> Self {
        Self {
            id: run.id,
            name: run.name,
            status_label: run.current_status.to_string(),
            status_class: status::css_class(run.current_status),
            initiated_by: run.initiated_by,
            started_at: run.started_at.to_rfc3339(),
        }
    }
}

/// The home page's recent-runs table: the current page of rows plus enough state to render
/// Prev/Next pagination links. Carries `initiated_by`/`started_within` too (not just the page's
/// template fields) so it can build those links itself with correct percent-encoding.
pub struct RunListView {
    pub rows: Vec<RunListRowView>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    initiated_by: String,
    started_within: String,
}

impl RunListView {
    pub fn new(
        response: TestRunListResponse,
        limit: i64,
        offset: i64,
        initiated_by: String,
        started_within: String,
    ) -> Self {
        Self {
            rows: response
                .runs
                .into_iter()
                .map(RunListRowView::from)
                .collect(),
            total: response.total,
            limit,
            offset,
            initiated_by,
            started_within,
        }
    }

    pub fn has_prev(&self) -> bool {
        self.offset > 0
    }

    /// Whether there's a further page beyond what's shown. Derived from how many rows actually
    /// came back (`rows.len()`), not the requested `limit` — if `offset` landed past the end (a
    /// stale bookmark, or rows pruned since a pagination link was generated), `rows` is empty and
    /// this correctly reports no next page rather than trusting a now-bogus `offset`.
    pub fn has_next(&self) -> bool {
        self.offset + (self.rows.len() as i64) < self.total
    }

    /// The Prev link's href, already percent-encoding `initiated_by`/`started_within` — built here
    /// rather than interpolated in the template, since Askama's HTML-escaping alone doesn't
    /// percent-encode query values (a name containing `&` or `#` would otherwise corrupt the link).
    pub fn prev_href(&self) -> String {
        self.href_with_offset(self.prev_offset())
    }

    pub fn next_href(&self) -> String {
        self.href_with_offset(self.offset + self.limit)
    }

    /// One page back — but if the current page is empty because `offset` overshot the last page,
    /// a plain `offset - limit` step could still land past the end. Jump straight to the last page
    /// that actually has rows on it instead, so Prev always recovers in one click.
    fn prev_offset(&self) -> i64 {
        if self.rows.is_empty() && self.total > 0 {
            ((self.total - 1) / self.limit) * self.limit
        } else {
            (self.offset - self.limit).max(0)
        }
    }

    fn href_with_offset(&self, offset: i64) -> String {
        let mut qs = form_urlencoded::Serializer::new(String::new());
        if !self.initiated_by.is_empty() {
            qs.append_pair("initiated_by", &self.initiated_by);
        }
        if !self.started_within.is_empty() {
            qs.append_pair("started_within", &self.started_within);
        }
        qs.append_pair("offset", &offset.to_string());
        format!("/ui?{}", qs.finish())
    }

    /// e.g. "1-20 of 137". `None` when there's nothing to summarize — no matching runs at all, or
    /// `offset` landed past the last page — so the template shows `empty_message` instead rather
    /// than both a range and a "no runs" row at once.
    pub fn showing_range(&self) -> Option<String> {
        if self.rows.is_empty() {
            return None;
        }
        let last = self.offset + self.rows.len() as i64;
        Some(format!("{}-{} of {}", self.offset + 1, last, self.total))
    }

    /// The message shown in the table in place of rows when there's nothing on this page: either
    /// no runs match the filters at all, or `offset` landed past the last page of an otherwise
    /// non-empty result set.
    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.total == 0 {
            Some("No runs match these filters.")
        } else {
            Some("No runs on this page.")
        }
    }
}

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
        links::sample_config,
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

        assert_eq!(RunView::new(run, now, &sample_config()).completed_at, None);
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

        assert_eq!(
            RunView::new(run, now, &sample_config()).completed_at,
            Some(now.to_rfc3339())
        );
    }

    #[test]
    fn run_template_carries_the_poll_trigger_while_non_terminal() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Running),
            Utc::now(),
            &sample_config(),
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
    fn run_template_links_each_execution_to_its_gcp_logs_and_grafana_dashboard() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Running),
            Utc::now(),
            &sample_config(),
        );
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
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Running),
            Utc::now(),
            &sample_config(),
        );
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
    fn execution_template_omits_the_back_link_when_the_run_id_is_unknown() {
        let mut execution = sample_execution(Uuid::from_u128(1), Uuid::from_u128(2));
        execution.test_run_id = None;
        let view = ExecutionDetailView::new(execution, &sample_config());
        let body = ExecutionTemplate { execution: view }
            .render()
            .expect("template renders");

        assert!(!body.contains("Back to run"));
    }

    /// Builds a `RunListView` with `n_rows` placeholder rows out of `total` matching runs, at the
    /// given `limit`/`offset`.
    fn list_view(n_rows: usize, total: i64, limit: i64, offset: i64) -> RunListView {
        list_view_with_filters(n_rows, total, limit, offset, "", "")
    }

    fn list_view_with_filters(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        initiated_by: &str,
        started_within: &str,
    ) -> RunListView {
        let response = TestRunListResponse {
            runs: vec![TestRunSummary::default(); n_rows],
            total,
        };
        RunListView::new(
            response,
            limit,
            offset,
            initiated_by.to_owned(),
            started_within.to_owned(),
        )
    }

    #[test]
    fn has_next_true_when_more_rows_remain() {
        let view = list_view(20, 137, 20, 0);
        assert!(view.has_next());
    }

    #[test]
    fn has_next_false_on_the_last_full_page() {
        let view = list_view(20, 20, 20, 0);
        assert!(!view.has_next());
    }

    #[test]
    fn has_next_false_when_offset_landed_past_the_end() {
        // `total` shrank (or a stale link was followed) since this offset was generated.
        let view = list_view(0, 15, 20, 40);
        assert!(!view.has_next());
        assert!(view.has_prev());
    }

    #[test]
    fn has_prev_false_on_the_first_page() {
        let view = list_view(20, 137, 20, 0);
        assert!(!view.has_prev());
    }

    #[test]
    fn prev_href_steps_back_by_limit_normally() {
        let view = list_view(20, 137, 20, 40);
        assert_eq!(view.prev_href(), "/ui?offset=20");
    }

    #[test]
    fn prev_href_jumps_to_the_last_real_page_when_offset_overshot() {
        // total=137, limit=20 -> last real page starts at offset 120 (rows 121-137).
        let view = list_view(0, 137, 20, 500);
        assert_eq!(view.prev_href(), "/ui?offset=120");
    }

    #[test]
    fn empty_message_is_none_when_rows_are_present() {
        let view = list_view(20, 137, 20, 0);
        assert_eq!(view.empty_message(), None);
    }

    #[test]
    fn empty_message_distinguishes_no_matches_from_past_the_end() {
        assert_eq!(
            list_view(0, 0, 20, 0).empty_message(),
            Some("No runs match these filters.")
        );
        assert_eq!(
            list_view(0, 137, 20, 500).empty_message(),
            Some("No runs on this page.")
        );
    }

    #[test]
    fn showing_range_is_none_when_there_are_no_rows() {
        assert_eq!(list_view(0, 0, 20, 0).showing_range(), None);
        assert_eq!(list_view(0, 137, 20, 500).showing_range(), None);
    }

    #[test]
    fn showing_range_formats_the_current_page() {
        let view = list_view(17, 137, 20, 120);
        assert_eq!(view.showing_range(), Some("121-137 of 137".to_owned()));
    }

    #[test]
    fn hrefs_percent_encode_special_characters_in_filters() {
        let view = list_view_with_filters(20, 137, 20, 40, "a&b#c d", "day");
        let href = view.prev_href();

        assert!(
            href.contains("initiated_by=a%26b%23c+d"),
            "expected percent-encoded initiated_by, got {href}"
        );
        assert!(href.contains("started_within=day"));
    }
}
