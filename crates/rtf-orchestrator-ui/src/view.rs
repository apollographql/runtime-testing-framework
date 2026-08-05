use crate::links::{LinksConfig, gcp_logs, grafana};
use crate::status;
use chrono::{DateTime, Utc};
use humantime::{format_duration, format_rfc3339_seconds};
use rep_orchestrator_shared::known_test_plan::{KnownTestPlanListResponse, KnownTestPlanSummary};
use rep_orchestrator_shared::status::{Status, StatusUpdate};
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunListResponse, TestRunSummary};
use std::time::{Duration, SystemTime};
use url::form_urlencoded;
use uuid::Uuid;

/// Upper bound on how long a non-terminal run is auto-refreshed. A run that never reaches a
/// terminal state (e.g. a stuck orchestrator) would otherwise be polled forever; past this age the
/// UI stops polling and the user can refresh manually.
const MAX_POLL_AGE_SECS: i64 = 60 * 60;

/// Renders a timestamp as RFC 3339 at second precision (e.g. `2026-07-27T14:23:01Z`), the format
/// every timestamp on the UI is shown in.
fn format_rfc3339(dt: DateTime<Utc>) -> String {
    let seconds_since_epoch = dt.timestamp().max(0) as u64;
    let system_time = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds_since_epoch);

    format_rfc3339_seconds(system_time).to_string()
}

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
    /// How many executions this run has in total, unaffected by `execution_status_filter` — shown
    /// next to the table heading so filtering down to a status doesn't read as "the run only has
    /// this many executions."
    pub total_executions: usize,
    /// The `current_status` value (its `Display` string, e.g. `"RUNNING"`) executions are filtered
    /// down to, or empty for no filter. Kept as a string, not a parsed [`Status`], since its only
    /// uses are re-populating the `<select>` and round-tripping through URLs.
    pub execution_status_filter: String,
    /// How many executions are in each status, in lifecycle order, excluding statuses no execution
    /// currently has. Always reflects every execution, regardless of `execution_status_filter`.
    pub status_breakdown: Vec<StatusCountView>,
    /// Executions matching `execution_status_filter` (all of them, if empty).
    pub executions: Vec<ExecutionView>,
}

impl RunView {
    /// Build the run view. `now` decides whether the region keeps polling and anchors the elapsed
    /// time and "last updated" indicator. `links_cfg` supplies the GCP/Grafana deep-link
    /// configuration for each execution row. `execution_status_filter` restricts the executions
    /// table to rows whose status matches exactly (empty means no filter).
    pub fn new(
        run: TestRunSummary,
        now: DateTime<Utc>,
        links_cfg: &LinksConfig,
        execution_status_filter: String,
    ) -> Self {
        // Trust `current_status`, not just the presence of `completed_at`, to decide whether the run
        // is actually done — a run that is still in progress should never show a completion time,
        // even if the wire data is momentarily inconsistent.
        let completed_at = run
            .completed_at
            .filter(|_| run.current_status.is_terminal());
        let end = completed_at.unwrap_or(now);
        let total_executions = run.executions.len();
        let status_breakdown = status_breakdown(&run.executions);

        Self {
            id: run.id,
            name: run.name,
            status_label: run.current_status.to_string(),
            status_class: status::css_class(run.current_status),
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

    /// The URL the executions table's htmx auto-poll and manual "Refresh now" button fetch from —
    /// this run's status page, carrying `execution_status_filter` forward so a refresh doesn't
    /// silently drop the active filter.
    pub fn poll_url(&self) -> String {
        if self.execution_status_filter.is_empty() {
            format!("/ui/run/{}", self.id)
        } else {
            let mut qs = form_urlencoded::Serializer::new(String::new());
            qs.append_pair("execution_status", &self.execution_status_filter);
            format!("/ui/run/{}?{}", self.id, qs.finish())
        }
    }

    /// The message shown in the executions table in place of rows when there's nothing to show:
    /// either the run genuinely has no executions yet, or a status filter matched none of them.
    pub fn executions_empty_message(&self) -> &'static str {
        if self.execution_status_filter.is_empty() {
            "No executions yet."
        } else {
            "No executions match this filter."
        }
    }
}

/// Every [`Status`] variant, in lifecycle order — the order the run's status breakdown and the
/// execution-status filter's `<select>` present statuses in.
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

/// One row in the run's status-breakdown summary.
pub struct StatusCountView {
    pub status_label: String,
    pub status_class: &'static str,
    pub count: usize,
}

/// How many `executions` are in each status, in lifecycle order, omitting statuses none of them are
/// in — a run with no `Unrunnable` executions shouldn't show an "Unrunnable: 0" row.
fn status_breakdown(executions: &[TestExecutionSummary]) -> Vec<StatusCountView> {
    STATUS_LIFECYCLE_ORDER
        .into_iter()
        .filter_map(|candidate| {
            let count = executions
                .iter()
                .filter(|execution| execution.current_status == candidate)
                .count();

            (count > 0).then(|| StatusCountView {
                status_label: candidate.to_string(),
                status_class: status::css_class(candidate),
                count,
            })
        })
        .collect()
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
            started_at: format_rfc3339(execution.started_at),
            updated_at: format_rfc3339(execution.updated_at),
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
            started_at: format_rfc3339(run.started_at),
        }
    }
}

enum RunListScope {
    Filtered {
        initiated_by: String,
        started_within: String,
    },
    KnownTestPlan(Uuid),
}

/// The recent-runs table shared by the home page and a known test plan's detail page: the current
/// page of rows plus enough state to render Prev/Next pagination links.
pub struct RunListView {
    pub rows: Vec<RunListRowView>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    scope: RunListScope,
}

impl RunListView {
    pub fn new(
        response: TestRunListResponse,
        limit: i64,
        offset: i64,
        initiated_by: String,
        started_within: String,
    ) -> Self {
        Self::build(
            response,
            limit,
            offset,
            RunListScope::Filtered {
                initiated_by,
                started_within,
            },
        )
    }

    pub fn for_known_test_plan(
        response: TestRunListResponse,
        limit: i64,
        offset: i64,
        plan_uuid: Uuid,
    ) -> Self {
        Self::build(
            response,
            limit,
            offset,
            RunListScope::KnownTestPlan(plan_uuid),
        )
    }

    fn build(response: TestRunListResponse, limit: i64, offset: i64, scope: RunListScope) -> Self {
        Self {
            rows: response
                .runs
                .into_iter()
                .map(RunListRowView::from)
                .collect(),
            total: response.total,
            limit,
            offset,
            scope,
        }
    }

    pub fn has_prev(&self) -> bool {
        self.offset > 0
    }

    pub fn has_next(&self) -> bool {
        self.offset + (self.rows.len() as i64) < self.total
    }

    pub fn prev_href(&self) -> String {
        self.href_with_offset(self.prev_offset())
    }

    pub fn next_href(&self) -> String {
        self.href_with_offset(self.offset + self.limit)
    }

    fn prev_offset(&self) -> i64 {
        if self.rows.is_empty() && self.total > 0 {
            ((self.total - 1) / self.limit) * self.limit
        } else {
            (self.offset - self.limit).max(0)
        }
    }

    fn href_with_offset(&self, offset: i64) -> String {
        match &self.scope {
            RunListScope::KnownTestPlan(plan_uuid) => {
                format!("/ui/test-plan/{plan_uuid}?offset={offset}")
            }
            RunListScope::Filtered {
                initiated_by,
                started_within,
            } => {
                let mut qs = form_urlencoded::Serializer::new(String::new());
                if !initiated_by.is_empty() {
                    qs.append_pair("initiated_by", initiated_by);
                }
                if !started_within.is_empty() {
                    qs.append_pair("started_within", started_within);
                }
                qs.append_pair("offset", &offset.to_string());
                format!("/ui?{}", qs.finish())
            }
        }
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

    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.total == 0 {
            Some(match self.scope {
                RunListScope::KnownTestPlan(_) => "This test plan has no runs yet.",
                RunListScope::Filtered { .. } => "No runs match these filters.",
            })
        } else {
            Some("No runs on this page.")
        }
    }
}

/// One row in the known-test-plans table.
pub struct KnownTestPlanRowView {
    pub uuid: Uuid,
    pub name: String,
    /// Empty when the plan has no description, so the template can render it plainly.
    pub description: String,
    pub org: String,
    pub repo: String,
    pub path: String,
    /// Link to the test plan file on GitHub.
    pub github_url: String,
}

impl From<KnownTestPlanSummary> for KnownTestPlanRowView {
    fn from(plan: KnownTestPlanSummary) -> Self {
        Self {
            github_url: known_test_plan_github_url(&plan.org, &plan.repo, &plan.path),
            uuid: plan.uuid,
            name: plan.name,
            description: plan.description.unwrap_or_default(),
            org: plan.org,
            repo: plan.repo,
            path: plan.path,
        }
    }
}

/// The test plan file's URL on GitHub. Resolved against `blob/HEAD/...` (the repo's default
/// branch) rather than a specific ref, since no ref is persisted on a [`KnownTestPlanSummary`] —
/// only the run it produced (via `known_test_plan_run.git_sha`) records the ref actually used.
fn known_test_plan_github_url(org: &str, repo: &str, path: &str) -> String {
    format!("https://github.com/{org}/{repo}/blob/HEAD/{path}")
}

/// The known-test-plans page's table: the current page of rows plus enough state to render
/// Prev/Next pagination links, mirroring [`RunListView`].
pub struct KnownTestPlanListView {
    pub rows: Vec<KnownTestPlanRowView>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

impl KnownTestPlanListView {
    pub fn new(response: KnownTestPlanListResponse, limit: i64, offset: i64) -> Self {
        Self {
            rows: response
                .test_plans
                .into_iter()
                .map(KnownTestPlanRowView::from)
                .collect(),
            total: response.total,
            limit,
            offset,
        }
    }

    pub fn has_prev(&self) -> bool {
        self.offset > 0
    }

    /// See [`RunListView::has_next`] for why this is derived from `rows.len()`, not `limit`.
    pub fn has_next(&self) -> bool {
        self.offset + (self.rows.len() as i64) < self.total
    }

    pub fn prev_href(&self) -> String {
        self.href_with_offset(self.prev_offset())
    }

    pub fn next_href(&self) -> String {
        self.href_with_offset(self.offset + self.limit)
    }

    /// See [`RunListView::prev_offset`] for why this jumps straight to the last page with rows on
    /// it rather than naively stepping back by `limit`.
    fn prev_offset(&self) -> i64 {
        if self.rows.is_empty() && self.total > 0 {
            ((self.total - 1) / self.limit) * self.limit
        } else {
            (self.offset - self.limit).max(0)
        }
    }

    fn href_with_offset(&self, offset: i64) -> String {
        let mut qs = form_urlencoded::Serializer::new(String::new());
        qs.append_pair("offset", &offset.to_string());
        format!("/ui/test-plans?{}", qs.finish())
    }

    /// e.g. "1-20 of 42". `None` when there's nothing to summarize.
    pub fn showing_range(&self) -> Option<String> {
        if self.rows.is_empty() {
            return None;
        }
        let last = self.offset + self.rows.len() as i64;
        Some(format!("{}-{} of {}", self.offset + 1, last, self.total))
    }

    /// The message shown in the table in place of rows when there's nothing on this page.
    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.total == 0 {
            Some("No known test plans are registered.")
        } else {
            Some("No known test plans on this page.")
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

        assert_eq!(
            RunView::new(run, now, &sample_config(), String::new()).completed_at,
            None
        );
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
            RunView::new(run, now, &sample_config(), String::new()).completed_at,
            Some(format_rfc3339(now))
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
    fn run_template_links_each_execution_to_its_gcp_logs_and_grafana_dashboard() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Running),
            Utc::now(),
            &sample_config(),
            String::new(),
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
            String::new(),
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
    fn run_template_shows_the_execution_count() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = RunView::new(
            sample_summary(run_id, ex_id, Status::Running),
            Utc::now(),
            &sample_config(),
            String::new(),
        );
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(
            body.contains("<h3>Executions (1)</h3>"),
            "expected the single sample execution to be counted next to the table heading, got: {body}"
        );
    }

    /// A run with two executions in different terminal statuses, for exercising the
    /// `execution_status_filter`.
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

    #[test]
    fn run_template_renders_the_filter_select_with_the_current_value_chosen() {
        let run_id = Uuid::from_u128(1);
        let run = RunView::new(
            run_with_mixed_execution_statuses(run_id),
            Utc::now(),
            &sample_config(),
            "FAILED".to_owned(),
        );
        let body = RunTemplate { run }.render().expect("template renders");

        assert!(
            body.contains("<option value=\"FAILED\" selected>FAILED</option>"),
            "expected the active filter to be pre-selected, got: {body}"
        );
        assert!(
            body.contains("exec-broke"),
            "matching execution should still render"
        );
        assert!(
            !body.contains("exec-ok"),
            "non-matching execution should be filtered out of the table"
        );
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
            .map(|row| (row.status_label.as_str(), row.count))
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
            "statuses with no executions should not get a status pill anywhere on the page \
             (the filter <select> always lists UNRUNNABLE as an option, so that text alone isn't \
             a safe check)"
        );
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

    /// Builds a `KnownTestPlanListView` with `n_rows` placeholder rows out of `total` matching
    /// plans, at the given `limit`/`offset`.
    fn known_test_plan_list_view(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
    ) -> KnownTestPlanListView {
        let response = KnownTestPlanListResponse {
            test_plans: (0..n_rows)
                .map(|_| KnownTestPlanSummary {
                    uuid: Uuid::new_v4(),
                    name: "a-known-plan".to_owned(),
                    description: None,
                    org: "apollographql".to_owned(),
                    repo: "runtime-testing-framework".to_owned(),
                    path: "test-plans/example.yaml".to_owned(),
                })
                .collect(),
            total,
        };
        KnownTestPlanListView::new(response, limit, offset)
    }

    #[test]
    fn known_test_plan_row_defaults_a_missing_description_to_empty() {
        let row = KnownTestPlanRowView::from(KnownTestPlanSummary {
            uuid: Uuid::new_v4(),
            name: "plan".to_owned(),
            description: None,
            org: "org".to_owned(),
            repo: "repo".to_owned(),
            path: "path.yaml".to_owned(),
        });

        assert_eq!(row.description, "");
    }

    #[test]
    fn known_test_plan_row_builds_a_github_blob_url_against_head() {
        let row = KnownTestPlanRowView::from(KnownTestPlanSummary {
            uuid: Uuid::new_v4(),
            name: "plan".to_owned(),
            description: None,
            org: "apollographql".to_owned(),
            repo: "runtime-testing-framework".to_owned(),
            path: "test-plans/example.yaml".to_owned(),
        });

        assert_eq!(
            row.github_url,
            "https://github.com/apollographql/runtime-testing-framework/blob/HEAD/test-plans/example.yaml"
        );
    }

    #[test]
    fn known_test_plan_list_has_next_true_when_more_rows_remain() {
        let view = known_test_plan_list_view(20, 42, 20, 0);
        assert!(view.has_next());
    }

    #[test]
    fn known_test_plan_list_has_next_false_on_the_last_full_page() {
        let view = known_test_plan_list_view(20, 20, 20, 0);
        assert!(!view.has_next());
    }

    #[test]
    fn known_test_plan_list_has_next_false_when_offset_landed_past_the_end() {
        let view = known_test_plan_list_view(0, 15, 20, 40);
        assert!(!view.has_next());
        assert!(view.has_prev());
    }

    #[test]
    fn known_test_plan_list_has_prev_false_on_the_first_page() {
        let view = known_test_plan_list_view(20, 42, 20, 0);
        assert!(!view.has_prev());
    }

    #[test]
    fn known_test_plan_list_prev_href_steps_back_by_limit_normally() {
        let view = known_test_plan_list_view(20, 137, 20, 40);
        assert_eq!(view.prev_href(), "/ui/test-plans?offset=20");
    }

    #[test]
    fn known_test_plan_list_prev_href_jumps_to_the_last_real_page_when_offset_overshot() {
        // total=137, limit=20 -> last real page starts at offset 120 (rows 121-137).
        let view = known_test_plan_list_view(0, 137, 20, 500);
        assert_eq!(view.prev_href(), "/ui/test-plans?offset=120");
    }

    #[test]
    fn known_test_plan_list_empty_message_is_none_when_rows_are_present() {
        let view = known_test_plan_list_view(20, 42, 20, 0);
        assert_eq!(view.empty_message(), None);
    }

    #[test]
    fn known_test_plan_list_empty_message_distinguishes_no_matches_from_past_the_end() {
        assert_eq!(
            known_test_plan_list_view(0, 0, 20, 0).empty_message(),
            Some("No known test plans are registered.")
        );
        assert_eq!(
            known_test_plan_list_view(0, 137, 20, 500).empty_message(),
            Some("No known test plans on this page.")
        );
    }

    #[test]
    fn known_test_plan_list_showing_range_is_none_when_there_are_no_rows() {
        assert_eq!(known_test_plan_list_view(0, 0, 20, 0).showing_range(), None);
        assert_eq!(
            known_test_plan_list_view(0, 137, 20, 500).showing_range(),
            None
        );
    }

    #[test]
    fn known_test_plan_list_showing_range_formats_the_current_page() {
        let view = known_test_plan_list_view(17, 137, 20, 120);
        assert_eq!(view.showing_range(), Some("121-137 of 137".to_owned()));
    }

    /// Builds a `RunListView` scoped to an arbitrary known test plan (via
    /// [`RunListView::for_known_test_plan`]), with `n_rows` placeholder rows out of `total`
    /// matching runs, at the given `limit`/`offset`.
    fn run_list_view_for_known_test_plan(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
    ) -> RunListView {
        let response = TestRunListResponse {
            runs: vec![TestRunSummary::default(); n_rows],
            total,
        };
        RunListView::for_known_test_plan(response, limit, offset, Uuid::from_u128(1))
    }

    #[test]
    fn run_list_for_known_test_plan_has_next_true_when_more_rows_remain() {
        let view = run_list_view_for_known_test_plan(20, 137, 20, 0);
        assert!(view.has_next());
    }

    #[test]
    fn run_list_for_known_test_plan_has_next_false_on_the_last_full_page() {
        let view = run_list_view_for_known_test_plan(20, 20, 20, 0);
        assert!(!view.has_next());
    }

    #[test]
    fn run_list_for_known_test_plan_has_next_false_when_offset_landed_past_the_end() {
        let view = run_list_view_for_known_test_plan(0, 15, 20, 40);
        assert!(!view.has_next());
        assert!(view.has_prev());
    }

    #[test]
    fn run_list_for_known_test_plan_has_prev_false_on_the_first_page() {
        let view = run_list_view_for_known_test_plan(20, 137, 20, 0);
        assert!(!view.has_prev());
    }

    #[test]
    fn run_list_for_known_test_plan_prev_href_steps_back_by_limit_normally() {
        let view = run_list_view_for_known_test_plan(20, 137, 20, 40);
        assert_eq!(
            view.prev_href(),
            format!("/ui/test-plan/{}?offset=20", Uuid::from_u128(1))
        );
    }

    #[test]
    fn run_list_for_known_test_plan_prev_href_jumps_to_the_last_real_page_when_offset_overshot() {
        // total=137, limit=20 -> last real page starts at offset 120 (rows 121-137).
        let view = run_list_view_for_known_test_plan(0, 137, 20, 500);
        assert_eq!(
            view.prev_href(),
            format!("/ui/test-plan/{}?offset=120", Uuid::from_u128(1))
        );
    }

    #[test]
    fn run_list_for_known_test_plan_next_href_is_scoped_to_the_plan() {
        let view = run_list_view_for_known_test_plan(20, 137, 20, 0);
        assert_eq!(
            view.next_href(),
            format!("/ui/test-plan/{}?offset=20", Uuid::from_u128(1))
        );
    }

    #[test]
    fn run_list_for_known_test_plan_empty_message_distinguishes_no_runs_from_past_the_end() {
        assert_eq!(
            run_list_view_for_known_test_plan(0, 0, 20, 0).empty_message(),
            Some("This test plan has no runs yet.")
        );
        assert_eq!(
            run_list_view_for_known_test_plan(0, 137, 20, 500).empty_message(),
            Some("No runs on this page.")
        );
    }

    #[test]
    fn run_list_for_known_test_plan_showing_range_formats_the_current_page() {
        let view = run_list_view_for_known_test_plan(17, 137, 20, 120);
        assert_eq!(view.showing_range(), Some("121-137 of 137".to_owned()));
    }
}
