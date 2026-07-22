use crate::{
    links::LinksConfig,
    orchestrator::{self, Client, Download, RunListFilter},
    templates::{
        ErrorTemplate, ExecutionNotFoundTemplate, ExecutionTemplate, IndexTemplate,
        RunNotFoundTemplate, RunTemplate,
    },
    view::{ExecutionDetailView, RunListView, RunView},
};
use askama::Template;
use axum::{
    Extension,
    extract::{Path, Query, State},
    http::header::{CONTENT_DISPOSITION, CONTENT_TYPE},
    response::{Html, IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunListResponse, TestRunSummary};
use reqwest::StatusCode;
use tracing::error;
use uuid::Uuid;

/// Render an askama template to a `(status, body)` pair. Template render errors are mapped to a 500
/// with an empty body — rendering only fails on genuine bugs (e.g. a missing field), not on bad
/// input, so there's no more specific message to show.
fn render_body<T: Template>(status: StatusCode, template: T) -> (StatusCode, String) {
    match template.render() {
        Ok(body) => (status, body),
        Err(error) => {
            error!(%error, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, String::new())
        }
    }
}

fn to_response((status, body): (StatusCode, String)) -> Response {
    (status, Html(body)).into_response()
}

const DEFAULT_LIMIT: i64 = 20;

/// Query params accepted by the home page.
#[derive(Debug, serde::Deserialize)]
pub struct IndexParams {
    initiated_by: Option<String>,
    started_within: Option<String>,
    offset: Option<i64>,
}

/// A coarse time-range preset rather than a raw datetime
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartedWithin {
    Hour,
    Day,
    Week,
    Month,
}

impl StartedWithin {
    /// Anything unrecognized is treated the same as absent rather than rejected.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "hour" => Some(Self::Hour),
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    fn as_duration(self) -> Duration {
        match self {
            Self::Hour => Duration::hours(1),
            Self::Day => Duration::days(1),
            Self::Week => Duration::days(7),
            Self::Month => Duration::days(30),
        }
    }
}

/// `GET /ui` — the run-id lookup box plus a filterable table of recent runs.
pub async fn index<C: Client>(
    State(orchestrator_client): State<C>,
    Query(params): Query<IndexParams>,
) -> Response {
    let offset = params.offset.unwrap_or(0).max(0);
    // Trim so a whitespace-only box (stray leading/trailing space) doesn't silently filter on a
    // value nothing will ever match — an empty result then genuinely means "no filter applied."
    let initiated_by = params
        .initiated_by
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    // Kept as the raw string, not re-derived from `StartedWithin` — parsing it decides the actual
    // `started_after` filter below, but the display/pagination-link copy doesn't need a second,
    // separately-materialized value; an unrecognized value just passes through unchanged (the
    // `<select>` has no matching `<option>` for it, so it renders as "Any time" regardless).
    let started_within = params.started_within.unwrap_or_default();

    let filter = RunListFilter {
        initiated_by: initiated_by.clone(),
        started_after: StartedWithin::parse(&started_within).map(|w| Utc::now() - w.as_duration()),
        limit: DEFAULT_LIMIT,
        offset,
    };

    let list = orchestrator_client.list_runs(&filter).await;

    to_response(index_body(
        list,
        DEFAULT_LIMIT,
        offset,
        initiated_by,
        started_within,
    ))
}

/// Maps the result of listing recent runs to a rendered `(status, body)` pair. A fetch failure
/// still renders the page — with the run-id lookup box intact and an inline error in place of the
/// table — since that box doesn't depend on the list endpoint at all.
fn index_body(
    result: Result<TestRunListResponse, orchestrator::Error>,
    limit: i64,
    offset: i64,
    initiated_by: String,
    started_within: String,
) -> (StatusCode, String) {
    match result {
        Ok(response) => render_body(
            StatusCode::OK,
            IndexTemplate {
                list: Some(RunListView::new(
                    response,
                    limit,
                    offset,
                    initiated_by.clone(),
                    started_within.clone(),
                )),
                list_error: None,
                initiated_by,
                started_within,
            },
        ),
        Err(error) => {
            error!(%error, "failed to list runs from orchestrator");
            render_body(
                StatusCode::OK,
                IndexTemplate {
                    initiated_by,
                    started_within,
                    list: None,
                    list_error: Some(
                        "Could not load recent runs from the orchestrator.".to_owned(),
                    ),
                },
            )
        }
    }
}

/// `GET /ui/health` — liveness/readiness probe for the standalone service.
pub async fn health() -> &'static str {
    "ok"
}

/// Maps the result of fetching a run summary to a rendered `(status, body)` pair: the banner plus
/// executions table on success, a "not found" page for an unknown id, or an error page for a fetch
/// failure. Kept free of the orchestrator [`Client`] so it can be tested directly against
/// hand-built results, without a mock client or handler plumbing.
fn run_status_body(
    id: Uuid,
    result: Result<Option<TestRunSummary>, orchestrator::Error>,
    now: DateTime<Utc>,
    links_cfg: &LinksConfig,
) -> (StatusCode, String) {
    match result {
        Ok(Some(summary)) => render_body(
            StatusCode::OK,
            RunTemplate {
                run: RunView::new(summary, now, links_cfg),
            },
        ),
        Ok(None) => render_body(
            StatusCode::NOT_FOUND,
            RunNotFoundTemplate { id: id.to_string() },
        ),
        Err(error) => {
            error!(%error, %id, "failed to fetch run summary from orchestrator");
            render_body(
                StatusCode::BAD_GATEWAY,
                ErrorTemplate {
                    message: "Could not load this run from the orchestrator.".to_owned(),
                },
            )
        }
    }
}

/// `GET /ui/run/{id}` — overall status banner plus the executions table.
///
/// `id` is extracted as a [`Uuid`], so a malformed id is rejected before it reaches the provider.
///
/// This route also serves htmx's poll: the `#run` region carries `hx-get` back to this same URL with
/// `hx-select="#run"`, so htmx fetches the full page and swaps in only that element rather than the
/// UI needing a separate fragment-only route. Non-2xx responses are not swapped by htmx, so a
/// transient orchestrator error (or a run that 404s mid-poll) simply leaves the previous content in
/// place and retries on the next tick.
pub async fn run_status<C: Client>(
    State(orchestrator_client): State<C>,
    Extension(links_cfg): Extension<LinksConfig>,
    Path(id): Path<Uuid>,
) -> Response {
    let result = orchestrator_client.run_summary(id).await;

    to_response(run_status_body(id, result, Utc::now(), &links_cfg))
}

/// Maps the result of fetching an execution summary to a rendered `(status, body)` pair: the detail
/// page on success, a "not found" page for an unknown id, or an error page for a fetch failure. Kept
/// free of the orchestrator [`Client`] so it can be tested directly against hand-built results,
/// without a mock client or handler plumbing.
fn execution_detail_body(
    execution_id: Uuid,
    result: Result<Option<TestExecutionSummary>, orchestrator::Error>,
    links_cfg: &LinksConfig,
) -> (StatusCode, String) {
    match result {
        Ok(Some(execution)) => render_body(
            StatusCode::OK,
            ExecutionTemplate {
                execution: ExecutionDetailView::new(execution, links_cfg),
            },
        ),
        Ok(None) => render_body(
            StatusCode::NOT_FOUND,
            ExecutionNotFoundTemplate {
                execution_id: execution_id.to_string(),
            },
        ),
        Err(error) => {
            error!(%error, %execution_id, "failed to fetch execution summary from orchestrator");
            render_body(
                StatusCode::BAD_GATEWAY,
                ErrorTemplate {
                    message: "Could not load this execution from the orchestrator.".to_owned(),
                },
            )
        }
    }
}

/// `GET /ui/execution/{eid}` — one execution's detail: its status-history timeline (newest-first),
/// exit code, and timestamps.
///
/// Fetches the execution status directly from the orchestrator rather than pulling the whole
/// parent run.
pub async fn execution_detail<C: Client>(
    State(orchestrator_client): State<C>,
    Extension(links_cfg): Extension<LinksConfig>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    let result = orchestrator_client.execution_summary(execution_id).await;

    to_response(execution_detail_body(execution_id, result, &links_cfg))
}

/// `GET /ui/execution/{eid}/log.txt`
///
/// Proxies the execution's log file from the orchestrator, so
/// the browser only ever talks to the UI's own origin.
pub async fn execution_log<C: Client>(
    State(orchestrator_client): State<C>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    download_response(
        orchestrator_client.execution_log(execution_id).await,
        "text/plain; charset=utf-8",
        format!("{execution_id}-log.txt"),
    )
}

/// `GET /ui/execution/{eid}/output.zip`
///
/// Proxies the execution's zipped output from the orchestrator.
pub async fn execution_output_zip<C: Client>(
    State(orchestrator_client): State<C>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    download_response(
        orchestrator_client.execution_output_zip(execution_id).await,
        "application/zip",
        format!("{execution_id}-output.zip"),
    )
}

/// Maps a [`Download`] outcome to an HTTP response: the file's bytes with a `Content-Disposition`
/// download header on success, or a plain-text response carrying the corresponding status
/// otherwise.
fn download_response(
    result: Result<Download, orchestrator::Error>,
    content_type: &'static str,
    filename: String,
) -> Response {
    match result {
        Ok(Download::Ready(bytes)) => (
            [
                (CONTENT_TYPE, content_type.to_owned()),
                (
                    CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(Download::NotFound) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Ok(Download::NotReady) => (StatusCode::CONFLICT, "output not ready yet").into_response(),
        Err(error) => {
            error!(%error, "failed to fetch download from orchestrator");
            (
                StatusCode::BAD_GATEWAY,
                "could not fetch download from orchestrator",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        links::sample_config,
        orchestrator::mocks::{MockClient, sample_execution, sample_summary},
    };
    use axum::body::to_bytes;
    use rep_orchestrator_shared::status::Status;

    async fn body_text(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("response body");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    #[test]
    fn index_renders_the_run_id_form() {
        let (status, body) = index_body(
            Ok(TestRunListResponse::default()),
            DEFAULT_LIMIT,
            0,
            String::new(),
            String::new(),
        );

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("<form"),
            "expected a form on the landing page"
        );
        assert!(
            body.contains("/ui/run/"),
            "form should navigate to /ui/run/{{id}}"
        );
    }

    #[test]
    fn index_body_renders_the_recent_runs_table() {
        let run = sample_summary(Uuid::from_u128(1), Uuid::from_u128(2), Status::Running);
        let (status, body) = index_body(
            Ok(TestRunListResponse {
                runs: vec![run],
                total: 1,
            }),
            DEFAULT_LIMIT,
            0,
            String::new(),
            String::new(),
        );

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("my-test-run"),
            "recent-runs row should render"
        );
    }

    #[test]
    fn index_body_still_renders_the_lookup_form_when_listing_fails() {
        let (status, body) = index_body(
            Err(orchestrator::Error::ListRuns {
                status: StatusCode::BAD_GATEWAY,
            }),
            DEFAULT_LIMIT,
            0,
            String::new(),
            String::new(),
        );

        assert_eq!(status, StatusCode::OK, "the page itself still renders");
        assert!(
            body.contains("<form"),
            "run-id lookup box should survive a list failure"
        );
        assert!(body.contains("Could not load recent runs"));
    }

    #[tokio::test]
    async fn index_trims_whitespace_from_initiated_by() {
        let resp = index(
            State(MockClient::with_test_run(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Status::Running,
            )),
            Query(IndexParams {
                initiated_by: Some("  testuser  ".to_owned()),
                started_within: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            body_text(resp).await.contains("value=\"testuser\""),
            "the trimmed value should repopulate the filter box"
        );
    }

    #[tokio::test]
    async fn index_falls_back_to_any_time_for_an_unrecognized_started_within() {
        let resp = index(
            State(MockClient::with_test_run(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Status::Running,
            )),
            Query(IndexParams {
                initiated_by: None,
                started_within: Some("decade".to_owned()),
                offset: None,
            }),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "an unrecognized preset should not reject the request"
        );
    }

    #[tokio::test]
    async fn health_returns_ok() {
        assert_eq!(health().await, "ok");
    }

    #[test]
    fn run_status_body_renders_banner_and_executions_for_a_known_run() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let (status, body) = run_status_body(
            run_id,
            Ok(Some(sample_summary(run_id, ex_id, Status::Running))),
            Utc::now(),
            &sample_config(),
        );

        assert_eq!(status, StatusCode::OK);
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
    fn run_status_body_renders_not_found_for_an_unknown_run() {
        let id = Uuid::from_u128(1);
        let (status, body) = run_status_body(id, Ok(None), Utc::now(), &sample_config());

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("not found"));
    }

    #[test]
    fn run_status_body_renders_error_when_the_fetch_fails() {
        let id = Uuid::from_u128(1);
        let (status, body) = run_status_body(
            id,
            Err(orchestrator::Error::TestRunStatus {
                status: StatusCode::BAD_GATEWAY,
                id,
            }),
            Utc::now(),
            &sample_config(),
        );

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("went wrong"));
    }

    #[test]
    fn execution_detail_body_renders_a_known_execution() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let (status, body) = execution_detail_body(
            ex_id,
            Ok(Some(sample_execution(run_id, ex_id))),
            &sample_config(),
        );

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("exec-alpha"), "execution name should render");
        assert!(
            body.contains(&format!("/ui/run/{run_id}")),
            "execution detail page should link back to its parent run"
        );
    }

    #[test]
    fn execution_detail_body_renders_not_found_for_an_unknown_execution() {
        let ex_id = Uuid::from_u128(2);
        let (status, body) = execution_detail_body(ex_id, Ok(None), &sample_config());

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("Execution not found"));
    }

    #[test]
    fn execution_detail_body_renders_error_when_the_fetch_fails() {
        let ex_id = Uuid::from_u128(2);
        let (status, body) = execution_detail_body(
            ex_id,
            Err(orchestrator::Error::TestExecutionStatus {
                status: StatusCode::BAD_GATEWAY,
                id: ex_id,
            }),
            &sample_config(),
        );

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("went wrong"));
    }

    #[tokio::test]
    async fn run_status_calls_the_client_and_renders_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = run_status(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Extension(sample_config()),
            Path(run_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            body_text(resp).await.contains("my-test-run"),
            "handler should render the client's run summary"
        );
    }

    #[tokio::test]
    async fn execution_detail_calls_the_client_and_renders_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = execution_detail(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Extension(sample_config()),
            Path(ex_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            body_text(resp).await.contains("exec-alpha"),
            "handler should render the client's execution summary"
        );
    }

    #[tokio::test]
    async fn execution_log_calls_the_client_and_streams_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = execution_log(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(ex_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "log contents");
    }

    #[tokio::test]
    async fn execution_output_zip_calls_the_client_and_streams_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = execution_output_zip(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(ex_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "output zip contents");
    }

    #[tokio::test]
    async fn download_response_returns_bytes_with_a_download_header_on_success() {
        let resp = download_response(
            Ok(Download::Ready(b"hello".to_vec())),
            "text/plain; charset=utf-8",
            "example.txt".to_owned(),
        );

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=\"example.txt\""
        );
        assert_eq!(body_text(resp).await, "hello");
    }

    #[tokio::test]
    async fn download_response_returns_404_when_not_found() {
        let resp = download_response(Ok(Download::NotFound), "text/plain", "f".to_owned());

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn download_response_returns_409_when_not_ready() {
        let resp = download_response(Ok(Download::NotReady), "text/plain", "f".to_owned());

        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn download_response_returns_502_when_the_fetch_fails() {
        let resp = download_response(
            Err(orchestrator::Error::Download {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                path: "/test-execution/1/log.txt".to_owned(),
            }),
            "text/plain",
            "f".to_owned(),
        );

        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }
}
