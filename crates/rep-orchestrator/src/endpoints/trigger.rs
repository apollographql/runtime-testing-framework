use crate::{conn, db::TestRun, error::Error, event_loop::SubmitError, state::ServerState};
use axum::{Json, extract::State};
use rep_orchestrator_shared::{payload::TriggerPayload, summary::TestRunSummary};

pub async fn handler(
    State(ServerState { eq_state, .. }): State<ServerState>,
    Json(payload): Json<TriggerPayload>,
) -> Result<Json<TestRunSummary>, Error> {
    let claim = eq_state
        .try_reserve_pending_executions(&payload.test_plan)
        .await
        .ok_or(Error::InsufficientCapacity)?;

    let (test_run, summary) = match init_run_and_build_summary(&payload.test_plan.name).await {
        Ok((tr, s)) => (tr, s),
        Err(e) => {
            eq_state.release_pending_execution_claim(claim).await;
            return Err(e);
        }
    };

    match eq_state
        .try_submit_test_plan(claim, test_run, payload)
        .await
    {
        Ok(()) => Ok(Json(summary)),

        // Our claim gets released for us by try_submit_test_plan in this case so we are safe to
        // just return the error.
        Err(SubmitError::ResolveChannelClosed) => Err(Error::ResolverChannelClosed),

        Err(SubmitError::InvalidClaim) => {
            unreachable!("claim is made using the submitted test plan")
        }
    }
}

async fn init_run_and_build_summary(name: &str) -> Result<(TestRun, TestRunSummary), Error> {
    let conn = conn!();
    let test_run = TestRun::init(name, conn).await?;
    let summary = test_run.clone().try_into_summary(conn).await?;

    Ok((test_run, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, test_helpers::TestServerState};
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

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_service_unavailable_when_queue_is_full() -> anyhow::Result<()> {
        let mut cfg = Config::get().clone();
        cfg.max_queued_executions = 0;

        let tss = TestServerState::new_with_config(&cfg);
        let payload = tss.minimal_trigger_payload();

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;

        assert_eq!(resp.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            tss.resolver_rx.is_empty(),
            "should not have submitted the test plan"
        );

        Ok(())
    }
}
