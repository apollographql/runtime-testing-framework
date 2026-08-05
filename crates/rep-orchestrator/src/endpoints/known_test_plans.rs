//! List Test Plans registered with the orchestrator, matching an optional name filter, or fetch a
//! single one by UUID.
use crate::{
    Error, Result, conn,
    db::{KnownTestPlan, KnownTestPlanFilter},
};
use axum::{
    Json,
    extract::{Path, Query},
};
use rep_orchestrator_shared::known_test_plan::{KnownTestPlanListResponse, KnownTestPlanSummary};
use serde::Deserialize;
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Deserialize)]
pub struct Params {
    name: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

pub async fn list_handler(Query(params): Query<Params>) -> Result<Json<KnownTestPlanListResponse>> {
    let conn = conn!();
    let filter = KnownTestPlanFilter { name: params.name };

    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = params.offset.unwrap_or(0).max(0);

    let test_plans = filter.matching(limit, offset, conn).await?;
    let total = filter.n_matching(conn).await?;

    Ok(Json(KnownTestPlanListResponse {
        test_plans: test_plans.into_iter().map(|tp| tp.into_summary()).collect(),
        total,
    }))
}

pub async fn by_uuid_handler(Path(uuid): Path<Uuid>) -> Result<Json<KnownTestPlanSummary>> {
    match KnownTestPlan::get_by_uuid(&uuid, conn!()).await? {
        Some(plan) => Ok(Json(plan.into_summary())),
        None => Err(Error::UnknownTestPlan {
            identifier: uuid.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestServerState;
    use reqwest::StatusCode;
    use uuid::Uuid;

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn list_handler_filters_by_name() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let name = unique("match-me");
        KnownTestPlan::register(&name, None, "org", "repo", &unique("path"), conn).await?;
        KnownTestPlan::register(
            &unique("not-this-one"),
            None,
            "org",
            "repo",
            &unique("path"),
            conn,
        )
        .await?;

        let resp = tss
            .test_server
            .get("/test-plan")
            .add_query_param("name", &name)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: KnownTestPlanListResponse = resp.json();

        assert_eq!(body.total, 1, "{body:?}");
        assert_eq!(body.test_plans[0].name, name);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn list_handler_respects_limit_and_offset() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let org = unique("pagination-org");
        for name in ["a", "b", "c"] {
            KnownTestPlan::register(&unique(name), None, &org, "repo", &unique("path"), conn)
                .await?;
        }

        let resp = tss
            .test_server
            .get("/test-plan")
            .add_query_param("limit", 1)
            .add_query_param("offset", 1)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: KnownTestPlanListResponse = resp.json();

        assert_eq!(body.test_plans.len(), 1, "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn by_uuid_handler_returns_200_for_known_test_plan() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let name = unique("plan");
        let known =
            KnownTestPlan::register(&name, None, "org", "repo", &unique("path"), conn!()).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-plan/{}", known.uuid()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: KnownTestPlanSummary = resp.json();

        assert_eq!(body.uuid, known.uuid());
        assert_eq!(body.name, name);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn by_uuid_handler_returns_404_for_unknown_test_plan() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let uuid = Uuid::new_v4();

        let resp = tss.test_server.get(&format!("/test-plan/{uuid}")).await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }
}
