use crate::{conn, db::TestRun, error::Error, event_loop::SubmitError, state::ServerState};
use axum::{Json, extract::State};
use rep_orchestrator_shared::{payload::TriggerPayload, summary::TestRunSummary};

pub async fn handler(
    State(state): State<ServerState>,
    Json(payload): Json<TriggerPayload>,
) -> Result<Json<TestRunSummary>, Error> {
    let conn = conn!();
    let test_run = TestRun::init(&payload.test_plan.name, conn).await?;

    let summary = test_run.clone().try_into_summary(conn).await?;

    match state.try_submit_test_plan(test_run, payload).await {
        Ok(()) => Ok(Json(summary)),
        Err(SubmitError::InsufficientCapacity(_)) => Err(Error::InsufficientCapacity),
        Err(SubmitError::ResolveChannelClosed(_)) => Err(Error::ResolverChannelClosed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestServerState;
    use rep_orchestrator_shared::status::Status;
    use reqwest::StatusCode;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_initialises_a_new_run_and_submits_to_resolver() -> anyhow::Result<()> {
        let mut tss = TestServerState::new();
        let payload = tss.minimal_trigger_payload();

        // The request itself should return 200
        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        // A test plan should have been submitted to the resolver
        let res = tss.resolver_rx.try_recv();
        assert!(res.is_ok(), "{res:?}");

        // The run should be initialising
        let summary: TestRunSummary = resp.json();
        assert_eq!(summary.current_status, Status::Initialising);

        // The run should be in the DB
        let maybe_run = TestRun::get_by_uuid(&summary.id, conn!()).await.unwrap();
        assert!(
            maybe_run.is_some(),
            "test run ID did not map to a known run in the DB"
        );

        Ok(())
    }
}
