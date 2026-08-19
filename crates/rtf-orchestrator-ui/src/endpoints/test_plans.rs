use crate::{
    endpoints::{render_body, to_response},
    orchestrator::{self, Client},
    templates::TestPlansTemplate,
    view::KnownTestPlanListView,
};
use axum::{
    extract::{Query, State},
    response::Response,
};
use reqwest::StatusCode;
use rtf_orchestrator_shared::known_test_plan::{
    KnownTestPlanListParams, KnownTestPlanListResponse,
};
use tracing::error;

const DEFAULT_LIMIT: i64 = 20;

/// Query params accepted by the known test plans page.
#[derive(Debug, serde::Deserialize)]
pub struct TestPlansParams {
    offset: Option<i64>,
}

/// `GET /ui/test-plans` — a paginated table of known test plans registered with the orchestrator.
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Query(params): Query<TestPlansParams>,
) -> Response {
    let offset = params.offset.unwrap_or(0).max(0);

    let list = orchestrator_client
        .list_known_test_plans(&KnownTestPlanListParams {
            name: None,
            limit: Some(DEFAULT_LIMIT),
            offset: Some(offset),
        })
        .await;

    to_response(test_plans_body(list, DEFAULT_LIMIT, offset))
}

/// Maps the result of listing known test plans to a rendered `(status, body)` pair. A fetch
/// failure still renders the page, with an inline error in place of the table.
fn test_plans_body(
    result: Result<KnownTestPlanListResponse, orchestrator::Error>,
    limit: i64,
    offset: i64,
) -> (StatusCode, String) {
    match result {
        Ok(response) => render_body(
            StatusCode::OK,
            TestPlansTemplate {
                list: Some(KnownTestPlanListView::new(response, limit, offset)),
                list_error: None,
            },
        ),
        Err(error) => {
            error!(%error, "failed to list known test plans from orchestrator");
            render_body(
                StatusCode::OK,
                TestPlansTemplate {
                    list: None,
                    list_error: Some(
                        "Could not load known test plans from the orchestrator.".to_owned(),
                    ),
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
        orchestrator::mocks::{MockClient, sample_known_test_plan},
    };
    use rtf_orchestrator_shared::status::Status;
    use uuid::Uuid;

    #[test]
    fn test_plans_body_renders_the_table() {
        // Row-field mapping (name, description, GitHub URL) is already covered by
        // `KnownTestPlanRowView` unit tests; this only needs to prove the response was threaded
        // through to `TestPlansTemplate`.
        let (status, body) = test_plans_body(
            Ok(KnownTestPlanListResponse {
                test_plans: vec![sample_known_test_plan(Uuid::new_v4())],
                total: 1,
            }),
            DEFAULT_LIMIT,
            0,
        );

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("my-known-test-plan"),
            "known test plan row should render"
        );
    }

    #[test]
    fn test_plans_body_still_renders_the_page_when_listing_fails() {
        let (status, body) = test_plans_body(
            Err(orchestrator::Error::ListKnownTestPlans {
                status: StatusCode::BAD_GATEWAY,
            }),
            DEFAULT_LIMIT,
            0,
        );

        assert_eq!(status, StatusCode::OK, "the page itself still renders");
        assert!(body.contains("Could not load known test plans"));
    }

    #[tokio::test]
    async fn handler_calls_the_client_and_renders_whatever_comes_back() {
        let resp = handler(
            State(MockClient::with_test_run(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Status::Running,
            )),
            Query(TestPlansParams { offset: None }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            body_text(resp).await.contains("my-known-test-plan"),
            "expected the mock client's sample known test plan to render"
        );
    }
}
