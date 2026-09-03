//! Fetch the status of a given [TestRun] by its UUID.
use crate::{Error, Result, conn, db::TestRun};
use axum::{
    Json,
    extract::{Path, Query},
};
use rtf_orchestrator_shared::summary::TestRunSummary;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct Params {
    #[serde(default = "include_executions")]
    with_executions: bool,
}

fn include_executions() -> bool {
    true
}

pub async fn handler(
    Path(id): Path<Uuid>,
    Query(Params { with_executions }): Query<Params>,
) -> Result<Json<TestRunSummary>> {
    let conn = conn!();

    match TestRun::get_by_uuid(&id, conn).await? {
        Some(tr) if with_executions => Ok(Json(tr.try_into_summary_with_executions(conn).await?)),
        Some(tr) => Ok(Json(tr.try_into_summary(conn).await?)),
        None => Err(Error::UnknownTestRun { id }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{ClusterId, KnownTestPlan, KnownTestPlanRun, Queryable},
        test_helpers::TestServerState,
    };
    use reqwest::StatusCode;
    use simple_test_case::test_case;

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_200_for_known_run() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let run_id = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn!())
            .await?
            .uuid();

        let resp = tss
            .test_server
            .get(&format!("/test-run/{run_id}/status"))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_omits_execution_test_run_id_and_status_history() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
        let run_id = tr.uuid();
        tr.init_execution("test", 0, conn).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-run/{run_id}/status"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let summary: TestRunSummary = resp.json();

        assert_eq!(summary.executions.len(), 1);
        assert_eq!(summary.executions[0].test_run_id, None);
        assert_eq!(summary.executions[0].test_plan_id, None);
        assert_eq!(summary.executions[0].status_history, Vec::new());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_includes_test_plan_id_when_run_is_linked_to_a_known_test_plan()
    -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let known =
            KnownTestPlan::register(&unique("run"), None, "org", "repo", &unique("path"), conn)
                .await?;
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
        let run_id = tr.uuid();
        tr.init_execution("test", 0, conn).await?;
        KnownTestPlanRun::link(known.id(), tr.id(), None, conn).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-run/{run_id}/status"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let summary: TestRunSummary = resp.json();

        assert_eq!(summary.test_plan_id, Some(known.uuid()));
        assert_eq!(
            summary.executions[0].test_plan_id, None,
            "nested executions should not repeat the parent run's test_plan_id"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_defaults_to_including_execution_details() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
        let run_id = tr.uuid();
        tr.init_execution("test", 0, conn).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-run/{run_id}/status"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let summary: TestRunSummary = resp.json();

        assert_eq!(summary.executions.len(), 1);

        Ok(())
    }

    #[test_case(true; "with executions")]
    #[test_case(false; "without executions")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_respects_with_executions_param(with_executions: bool) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
        let run_id = tr.uuid();
        tr.init_execution("test", 0, conn).await?;

        let resp = tss
            .test_server
            .get(&format!("/test-run/{run_id}/status"))
            .add_query_param("with_executions", with_executions)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let summary: TestRunSummary = resp.json();

        assert_eq!(
            summary.executions.len(),
            if with_executions { 1 } else { 0 }
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_404_for_unknown_run() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let run_id = Uuid::new_v4();

        let resp = tss
            .test_server
            .get(&format!("/test-run/{run_id}/status"))
            .await;
        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }
}
