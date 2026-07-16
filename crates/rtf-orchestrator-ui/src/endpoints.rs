use crate::{
    links::LinksConfig,
    orchestrator::{self, Client},
    templates::{
        ErrorTemplate, ExecutionNotFoundTemplate, ExecutionTemplate, IndexTemplate,
        RunNotFoundTemplate, RunTemplate,
    },
    view::{ExecutionDetailView, RunView},
};
use askama::Template;
use axum::{
    Extension,
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunSummary};
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

/// `GET /ui` — the run-id input form.
pub async fn index() -> Response {
    to_response(render_body(StatusCode::OK, IndexTemplate))
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
        let (status, body) = render_body(StatusCode::OK, IndexTemplate);

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
}
