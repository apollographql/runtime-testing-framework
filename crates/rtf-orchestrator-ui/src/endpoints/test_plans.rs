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

    to_response(render_body(
        StatusCode::OK,
        test_plans_template(list, DEFAULT_LIMIT, offset),
    ))
}

/// Maps the result of listing known test plans to the page's template. A fetch failure still
/// renders the page, with an inline error in place of the table.
fn test_plans_template(
    result: Result<KnownTestPlanListResponse, orchestrator::Error>,
    limit: i64,
    offset: i64,
) -> TestPlansTemplate {
    match result {
        Ok(response) => TestPlansTemplate {
            list: Some(KnownTestPlanListView::new(response, limit, offset)),
            list_error: None,
        },
        Err(error) => {
            error!(%error, "failed to list known test plans from orchestrator");
            TestPlansTemplate {
                list: None,
                list_error: Some(
                    "Could not load known test plans from the orchestrator.".to_owned(),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::mocks::sample_known_test_plan;
    use uuid::Uuid;

    #[test]
    fn test_plans_template_carries_the_listed_plans() {
        let t = test_plans_template(
            Ok(KnownTestPlanListResponse {
                test_plans: vec![sample_known_test_plan(Uuid::new_v4())],
                total: 1,
            }),
            DEFAULT_LIMIT,
            0,
        );

        let list = t.list.expect("the plans list should be present");
        assert_eq!(list.rows.len(), 1);
        assert_eq!(list.rows[0].name, "my-known-test-plan");
        assert_eq!(t.list_error, None);
    }

    #[test]
    fn test_plans_template_carries_an_inline_error_when_listing_fails() {
        let t = test_plans_template(
            Err(orchestrator::Error::ListKnownTestPlans {
                status: StatusCode::BAD_GATEWAY,
            }),
            DEFAULT_LIMIT,
            0,
        );

        assert!(t.list.is_none());
        assert!(t.list_error.is_some());
    }
}
