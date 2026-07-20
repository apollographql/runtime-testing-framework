//! Fetch the status of a given [TestRun] by its UUID.
use crate::{Error, Result, conn, db::TestRun};
use axum::{Json, extract::Path};
use rep_orchestrator_shared::summary::TestRunSummary;
use uuid::Uuid;

pub async fn handler(Path(id): Path<Uuid>) -> Result<Json<TestRunSummary>> {
    let conn = conn!();

    match TestRun::get_by_uuid(&id, conn).await? {
        Some(ex) => Ok(Json(ex.try_into_summary_with_executions(conn).await?)),
        None => Err(Error::UnknownTestRun { id }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestServerState;
    use reqwest::StatusCode;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_200_for_known_run() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let run_id = TestRun::init_unknown_initiator("test", None, conn!())
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
    async fn handler_leaves_execution_test_run_id_unset() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, conn).await?;
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
