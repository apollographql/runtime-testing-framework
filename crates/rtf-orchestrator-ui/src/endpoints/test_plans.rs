use crate::{
    endpoints::{render_body, to_response},
    orchestrator::{self, Client, OrchestratorClient},
    templates::TestPlansTemplate,
    view::KnownTestPlanListView,
};
use axum::{extract::Query, response::Response};
use reqwest::StatusCode;
use rtf_orchestrator_shared::{
    known_test_plan::{KnownTestPlanListParams, KnownTestPlanListResponse},
    whoami::WhoAmIResponse,
};
use tracing::error;

const DEFAULT_LIMIT: i64 = 20;

#[derive(Debug, serde::Deserialize)]
pub struct TestPlansParams {
    offset: Option<i64>,
}

pub async fn handler<C: Client>(
    OrchestratorClient(client): OrchestratorClient<C>,
    Query(params): Query<TestPlansParams>,
) -> Response {
    let offset = params.offset.unwrap_or(0).max(0);

    let list_params = KnownTestPlanListParams {
        name: None,
        limit: Some(DEFAULT_LIMIT),
        offset: Some(offset),
    };
    let (list, whoami) = tokio::join!(client.list_known_test_plans(&list_params), client.whoami());

    let whoami = whoami.unwrap_or_else(|error| {
        error!(%error, "failed to identify the current user from orchestrator");
        WhoAmIResponse::Unknown
    });

    to_response(render_body(
        StatusCode::OK,
        test_plans_template(list, &whoami, DEFAULT_LIMIT, offset),
    ))
}

fn test_plans_template(
    result: Result<KnownTestPlanListResponse, orchestrator::Error>,
    whoami: &WhoAmIResponse,
    limit: i64,
    offset: i64,
) -> TestPlansTemplate {
    match result {
        Ok(response) => TestPlansTemplate {
            list: Some(KnownTestPlanListView::new(response, whoami, limit, offset)),
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
    use crate::{
        orchestrator::mocks::sample_known_test_plan, preview::OWNER_EMAIL, view::ViewerBadge,
    };
    use rtf_orchestrator_shared::known_test_plan::Role;
    use uuid::Uuid;

    #[test]
    fn test_plans_template_carries_the_listed_plans_and_the_users_role() {
        let t = test_plans_template(
            Ok(KnownTestPlanListResponse {
                test_plans: vec![sample_known_test_plan(Uuid::new_v4())],
                total: 1,
            }),
            &WhoAmIResponse::User {
                email: OWNER_EMAIL.to_owned(),
            },
            DEFAULT_LIMIT,
            0,
        );

        let list = t.list.expect("the plans list should be present");
        assert_eq!(list.rows.len(), 1);
        assert_eq!(list.rows[0].plan.name, "my-known-test-plan");
        assert_eq!(list.rows[0].role, Some(ViewerBadge::for_role(Role::Owner)));
        assert_eq!(t.list_error, None);
    }

    #[test]
    fn test_plans_template_carries_an_inline_error_when_listing_fails() {
        let t = test_plans_template(
            Err(orchestrator::Error::ListKnownTestPlans {
                status: StatusCode::BAD_GATEWAY,
                message: "bad gateway".to_owned(),
            }),
            &WhoAmIResponse::Unknown,
            DEFAULT_LIMIT,
            0,
        );

        assert!(t.list.is_none());
        assert!(t.list_error.is_some());
    }
}
