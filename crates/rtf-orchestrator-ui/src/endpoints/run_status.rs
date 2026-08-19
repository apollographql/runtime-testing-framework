use crate::{
    endpoints::{render_body, to_response},
    links::LinksConfig,
    orchestrator::{self, Client},
    templates::{ErrorTemplate, RunNotFoundTemplate, RunTemplate},
    view::RunView,
};
use axum::{
    Extension,
    extract::{Path, Query, State},
    response::Response,
};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use rtf_orchestrator_shared::summary::TestRunSummary;
use tracing::error;
use uuid::Uuid;

/// Query params accepted by the run status page.
#[derive(Debug, Default, serde::Deserialize)]
pub struct RunParams {
    /// Restricts the executions table to this exact `current_status` `Display` value (e.g.
    /// `"FAILED"`); absent/empty means no filter.
    execution_status: Option<String>,
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
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Extension(links_cfg): Extension<LinksConfig>,
    Path(id): Path<Uuid>,
    Query(params): Query<RunParams>,
) -> Response {
    let result = orchestrator_client.run_summary(id).await;
    let execution_status_filter = params.execution_status.unwrap_or_default();

    to_response(run_status_body(
        id,
        result,
        Utc::now(),
        &links_cfg,
        execution_status_filter,
    ))
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
    execution_status_filter: String,
) -> (StatusCode, String) {
    match result {
        Ok(Some(summary)) => render_body(
            StatusCode::OK,
            RunTemplate {
                run: RunView::new(summary, now, links_cfg, execution_status_filter),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        endpoints::body_text,
        links::sample_config,
        orchestrator::mocks::{MockClient, sample_summary},
    };
    use rtf_orchestrator_shared::status::Status;

    #[test]
    fn run_status_body_renders_banner_and_executions_for_a_known_run() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let (status, body) = run_status_body(
            run_id,
            Ok(Some(sample_summary(run_id, ex_id, Status::Running))),
            Utc::now(),
            &sample_config(),
            String::new(),
        );

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("my-test-run"), "run name should render");
    }

    #[test]
    fn run_status_body_renders_not_found_for_an_unknown_run() {
        let id = Uuid::from_u128(1);
        let (status, body) =
            run_status_body(id, Ok(None), Utc::now(), &sample_config(), String::new());

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
            String::new(),
        );

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("went wrong"));
    }

    #[tokio::test]
    async fn run_status_calls_the_client_and_renders_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = handler(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            Extension(sample_config()),
            Path(run_id),
            Query(RunParams::default()),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            body_text(resp).await.contains("my-test-run"),
            "handler should render the client's run summary"
        );
    }
}
