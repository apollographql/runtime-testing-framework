use crate::{
    endpoints::{render_body, to_response},
    links::LinksConfig,
    orchestrator::{self, Client},
    templates::{ErrorTemplate, ExecutionNotFoundTemplate, ExecutionTemplate},
    view::ExecutionDetailView,
};
use axum::{
    Extension,
    extract::{Path, State},
    response::Response,
};
use reqwest::StatusCode;
use rtf_orchestrator_shared::summary::TestExecutionSummary;
use tracing::error;
use uuid::Uuid;

/// `GET /ui/execution/{eid}` — one execution's detail: its status-history timeline (newest-first),
/// exit code, and timestamps.
///
/// Fetches the execution status directly from the orchestrator rather than pulling the whole
/// parent run.
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Extension(links_cfg): Extension<LinksConfig>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    let result = orchestrator_client.execution_summary(execution_id).await;

    to_response(execution_detail_body(execution_id, result, &links_cfg))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        endpoints::body_text,
        links::sample_config,
        orchestrator::mocks::{MockClient, sample_execution},
    };
    use rtf_orchestrator_shared::status::Status;

    #[test]
    fn execution_detail_body_renders_a_known_execution() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let (status, body) = execution_detail_body(
            ex_id,
            Ok(Some(sample_execution(run_id, ex_id))),
            &sample_config(),
        );

        // The back-link and other field-by-field rendering are already covered by
        // `ExecutionDetailView` unit tests; this only needs to prove the summary was threaded
        // through to `ExecutionTemplate` rather than the not-found/error templates.
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("exec-alpha"), "execution name should render");
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
    async fn execution_detail_calls_the_client_and_renders_its_result() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = handler(
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
