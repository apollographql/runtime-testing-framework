use crate::{
    orchestrator::Client,
    templates::{
        ErrorTemplate, ExecutionNotFoundTemplate, ExecutionTemplate, IndexTemplate,
        RunNotFoundTemplate, RunTemplate,
    },
    view::{ExecutionDetailView, RunView},
};
use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use chrono::Utc;
use reqwest::StatusCode;
use tracing::error;
use uuid::Uuid;

/// Render an askama template to a `200 OK` HTML response.
pub(crate) fn render<T: Template>(template: T) -> Response {
    render_with_status(template, StatusCode::OK)
}

/// Render an askama template to an HTML response with the given status, mapping render errors to a
/// 500.
pub(crate) fn render_with_status<T: Template>(template: T, status: StatusCode) -> Response {
    match template.render() {
        Ok(body) => (status, Html(body)).into_response(),
        Err(error) => {
            error!(%error, "template render failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /ui` — the run-id input form.
pub async fn index() -> Response {
    render(IndexTemplate)
}

/// `GET /ui/health` — liveness/readiness probe for the standalone service.
pub async fn health() -> &'static str {
    "ok"
}

/// `GET /ui/run/{id}` — overall status banner plus the executions table.
///
/// `id` is extracted as a [`Uuid`], so a malformed id is rejected before it reaches the provider. A
/// well-formed but unknown id renders a "run not found" page; a fetch failure renders an error page
/// rather than a raw 500.
///
/// This route also serves htmx's poll: the `#run` region carries `hx-get` back to this same URL with
/// `hx-select="#run"`, so htmx fetches the full page and swaps in only that element rather than the
/// UI needing a separate fragment-only route. Non-2xx responses are not swapped by htmx, so a
/// transient orchestrator error (or a run that 404s mid-poll) simply leaves the previous content in
/// place and retries on the next tick.
pub async fn run_status<C: Client>(
    State(orchestrator_client): State<C>,
    Path(id): Path<Uuid>,
) -> Response {
    match orchestrator_client.run_summary(id).await {
        Ok(Some(summary)) => render(RunTemplate {
            run: RunView::new(summary, Utc::now()),
        }),
        Ok(None) => render_with_status(
            RunNotFoundTemplate { id: id.to_string() },
            StatusCode::NOT_FOUND,
        ),
        Err(error) => {
            error!(%error, %id, "failed to fetch run summary from orchestrator");
            render_with_status(
                ErrorTemplate {
                    message: "Could not load this run from the orchestrator.".to_owned(),
                },
                StatusCode::BAD_GATEWAY,
            )
        }
    }
}

/// `GET /ui/execution/{eid}` — one execution's detail: its status-history timeline (newest-first),
/// exit code, and timestamps.
///
/// Fetches the execution status directly from the orchestrator rather than pulling the whole
/// parent run, so an unknown execution renders an execution "not found" page (404) and a fetch
/// failure renders an error page rather than a raw 500.
pub async fn execution_detail<C: Client>(
    State(orchestrator_client): State<C>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    match orchestrator_client.execution_summary(execution_id).await {
        Ok(Some(execution)) => render(ExecutionTemplate {
            execution: ExecutionDetailView::new(execution),
        }),
        Ok(None) => render_with_status(
            ExecutionNotFoundTemplate {
                execution_id: execution_id.to_string(),
            },
            StatusCode::NOT_FOUND,
        ),
        Err(error) => {
            tracing::error!(%error, %execution_id, "failed to fetch execution summary from orchestrator");
            render_with_status(
                ErrorTemplate {
                    message: "Could not load this execution from the orchestrator.".to_owned(),
                },
                StatusCode::BAD_GATEWAY,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::mocks::MockClient;
    use axum::body::to_bytes;
    use rep_orchestrator_shared::{status::Status, summary::TestRunSummary};

    async fn body_text(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("response body");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    fn summary(status: Status) -> TestRunSummary {
        TestRunSummary {
            current_status: status,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn index_renders_the_run_id_form() {
        let resp = index().await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
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

    #[tokio::test]
    async fn non_terminal_run_polls_the_same_page_route_via_hx_select() {
        let run = RunView::new(summary(Status::Running), Utc::now());
        let id = run.id;
        let body = body_text(render(RunTemplate { run })).await;

        assert!(body.contains(&format!("hx-get=\"/ui/run/{id}\"")));
        assert!(body.contains("hx-select=\"#run\""));
        assert!(
            !body.contains("/fragment"),
            "polling should no longer hit a dedicated fragment route"
        );
    }

    #[tokio::test]
    async fn terminal_run_omits_the_poll_trigger() {
        let run = RunView::new(summary(Status::Successful), Utc::now());
        let body = body_text(render(RunTemplate { run })).await;

        // The manual "Refresh now" button always carries `hx-get`/`hx-select`; `hx-trigger` only
        // ever appears on the auto-poll attributes, so its absence is what proves polling stopped.
        assert!(!body.contains("hx-trigger"));
    }

    #[tokio::test]
    async fn page_renders_banner_and_executions_for_a_known_run() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = run_status(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(run_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("my-test-run"), "run name should render");
        assert!(body.contains("RUNNING"), "run status label should render");
        assert!(body.contains("exec-alpha"), "execution name should render");
        assert!(
            body.contains("SUCCESSFUL"),
            "execution status label should render"
        );
    }

    #[tokio::test]
    async fn page_links_each_execution_to_its_gcp_logs_and_grafana_dashboard() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = run_status(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(run_id),
        )
        .await;

        let body = body_text(resp).await;
        assert!(
            body.contains(&format!("resource.labels.namespace_name%3D%22{ex_id}%22")),
            "execution row should link to logs scoped to its own namespace"
        );
        assert!(
            body.contains(&format!("var-namespace={ex_id}")),
            "execution row should link to a Grafana dashboard scoped to its own namespace"
        );
    }

    #[tokio::test]
    async fn page_polls_while_the_run_is_non_terminal() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = run_status(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(Uuid::nil()),
        )
        .await;

        let body = body_text(resp).await;
        assert!(
            body.contains("hx-trigger=\"every 10s\""),
            "a running run should carry the poll trigger"
        );
        assert!(body.contains(&format!("hx-get=\"/ui/run/{run_id}\"")));
        assert!(
            body.contains("Auto-refreshing every 10s"),
            "a running run should show the auto-refresh indicator"
        );
    }

    #[tokio::test]
    async fn page_stops_polling_once_the_run_is_terminal() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = run_status(
            State(MockClient::with_test_run(run_id, ex_id, Status::Successful)),
            Path(run_id),
        )
        .await;

        let body = body_text(resp).await;
        assert!(
            !body.contains("hx-trigger"),
            "a terminal run should omit the poll trigger so refresh stops"
        );
        assert!(
            body.contains("Auto-refresh stopped"),
            "a terminal run should show that auto-refresh stopped"
        );
    }

    #[tokio::test]
    async fn page_shows_polish_indicators() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = run_status(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(run_id),
        )
        .await;

        let body = body_text(resp).await;
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

    #[tokio::test]
    async fn page_renders_not_found_for_an_unknown_run() {
        // The default MockClient returns None for the TestRunSummary
        // This mocks the NotFound scenario
        let id = Uuid::from_u128(1);
        let resp = run_status(State(MockClient::default()), Path(id)).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(body_text(resp).await.contains("not found"));
    }

    #[tokio::test]
    async fn page_renders_error_when_the_fetch_fails() {
        let id = Uuid::from_u128(1);
        let resp = run_status(
            State(MockClient::with_status_code(StatusCode::BAD_GATEWAY)),
            Path(id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        assert!(body_text(resp).await.contains("went wrong"));
    }

    #[tokio::test]
    async fn detail_renders_status_history_for_a_known_execution() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = execution_detail(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Path(ex_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
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

    #[tokio::test]
    async fn detail_renders_not_found_for_an_unknown_execution() {
        let ex_id = Uuid::from_u128(2);
        let resp = execution_detail(State(MockClient::default()), Path(ex_id)).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(body_text(resp).await.contains("Execution not found"));
    }

    #[tokio::test]
    async fn detail_renders_error_when_the_fetch_fails() {
        let ex_id = Uuid::from_u128(2);
        let resp = execution_detail(
            State(MockClient::with_status_code(StatusCode::BAD_GATEWAY)),
            Path(ex_id),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        assert!(body_text(resp).await.contains("went wrong"));
    }
}
