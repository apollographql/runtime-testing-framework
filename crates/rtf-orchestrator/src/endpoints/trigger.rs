use crate::{
    config::Config,
    conn,
    context::OrchestratorContext,
    db::{KnownTestPlan, KnownTestPlanRun, Queryable, TestRun},
    error::Error,
    event_loop::SubmitError,
    iap_identity::extract_authenticated_user_email,
    state::ServerState,
};
use axum::{Json, extract::State, http::HeaderMap};
use rtf_orchestrator_shared::{
    payload::{GitHubPayload, PreparedPayload, TriggerPayload},
    summary::TestRunSummary,
};
use serde_json::Value;
use tracing::{debug, info};

/// Bookkeeping for a run triggered from a registered [KnownTestPlan], recorded in the
/// `known_test_plan_run` junction table once the run itself has been created.
struct KnownTestPlanLink {
    known_test_plan_id: i32,
    git_sha: Option<String>,
}

pub async fn handler(
    State(ServerState { eq_state, .. }): State<ServerState>,
    headers: HeaderMap,
    Json(trigger_payload): Json<TriggerPayload>,
) -> Result<Json<TestRunSummary>, Error> {
    let (payload, ctx, known_link) = as_prepared_payload_with_context(trigger_payload).await?;

    debug!("validating test plan file provider usage");
    ctx.validate_environment_file_provider_usage(&payload.test_plan)
        .await?;

    debug!("reserving pending executions");
    let claim = eq_state
        .try_reserve_pending_executions(&payload.test_plan)
        .await
        .ok_or(Error::InsufficientCapacity)?;

    // Capture any runtime variable overrides before creating the run so the run is born with the
    // correct variables_id (NULL iff there were no overrides).
    let variables = payload
        .variables
        .as_ref()
        .map(|v| serde_json::to_value(v).expect("variables to serialize"));

    let initiated_by = extract_authenticated_user_email(&headers);

    debug!("initialising run");
    let (test_run, summary) =
        match init_run_and_build_summary(&payload.test_plan.name, variables, initiated_by).await {
            Ok((tr, s)) => (tr, s),
            Err(e) => {
                eq_state.release_pending_execution_claim(claim).await;
                return Err(e);
            }
        };

    if let Some(link) = known_link {
        debug!("linking run to known test plan");
        if let Err(e) = link_known_test_plan(&test_run, link).await {
            eq_state.release_pending_execution_claim(claim).await;
            return Err(e);
        }
    }

    debug!("submitting test plan");
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

async fn as_prepared_payload_with_context(
    trigger_payload: TriggerPayload,
) -> Result<
    (
        PreparedPayload,
        OrchestratorContext,
        Option<KnownTestPlanLink>,
    ),
    Error,
> {
    let mut known_link = None;

    let payload = match trigger_payload {
        TriggerPayload::Prepared(payload) => payload,

        TriggerPayload::GitHub(payload) => {
            info!("attempting to pull test plan details from GitHub");
            payload
                .into_prepared(Config::get().server_context())
                .await?
        }

        TriggerPayload::KnownTestPlanUuid(kp) => {
            info!("attempting to pull a known test plan by UUID");
            let known = KnownTestPlan::get_by_uuid(&kp.test_plan_uuid, conn!())
                .await?
                .ok_or_else(|| Error::UnknownTestPlan {
                    identifier: kp.test_plan_uuid.to_string(),
                })?;

            known_link = Some(KnownTestPlanLink {
                known_test_plan_id: known.id(),
                git_sha: kp.git_ref.clone(),
            });

            GitHubPayload {
                org: known.org().to_owned(),
                repo: known.repo().to_owned(),
                path: known.path().to_owned(),
                git_ref: kp.git_ref,
                variables: kp.variables,
            }
            .into_prepared(Config::get().server_context())
            .await?
        }

        TriggerPayload::KnownTestPlanName(kp) => {
            info!("attempting to pull a known test plan by name");
            let known = KnownTestPlan::get_by_name(&kp.test_plan_name, conn!())
                .await?
                .ok_or_else(|| Error::UnknownTestPlan {
                    identifier: kp.test_plan_name.clone(),
                })?;

            known_link = Some(KnownTestPlanLink {
                known_test_plan_id: known.id(),
                git_sha: kp.git_ref.clone(),
            });

            GitHubPayload {
                org: known.org().to_owned(),
                repo: known.repo().to_owned(),
                path: known.path().to_owned(),
                git_ref: kp.git_ref,
                variables: kp.variables,
            }
            .into_prepared(Config::get().server_context())
            .await?
        }
    };

    let ctx = OrchestratorContext::new_from_inlined_files(
        Config::get(),
        payload.relative_files.clone(),
        payload.custom_providers.clone(),
    );

    Ok((payload, ctx, known_link))
}

async fn link_known_test_plan(test_run: &TestRun, link: KnownTestPlanLink) -> Result<(), Error> {
    KnownTestPlanRun::link(
        link.known_test_plan_id,
        test_run.id(),
        link.git_sha.as_deref(),
        conn!(),
    )
    .await?;

    Ok(())
}

async fn init_run_and_build_summary(
    name: &str,
    variables: Option<Value>,
    initiated_by: Option<String>,
) -> Result<(TestRun, TestRunSummary), Error> {
    let conn = conn!();
    let test_run = TestRun::init(name, variables, initiated_by.as_deref(), conn).await?;
    let summary = test_run
        .clone()
        .try_into_summary_with_executions(conn)
        .await?;

    Ok((test_run, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, test_helpers::TestServerState};
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::status::Status;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_rejects_invalid_compose_file_provider_usage() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let payload = tss.minimal_invalid_compose_trigger_payload();

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
        assert!(
            tss.resolver_rx.is_empty(),
            "should not have submitted the test plan"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_initialises_a_new_run_and_submits_to_resolver() -> anyhow::Result<()> {
        let mut tss = TestServerState::new();
        let payload = tss.minimal_trigger_payload();

        // The request itself should return 200
        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .add_header(
                "x-goog-authenticated-user-email",
                "accounts.google.com:ci@my-project.iam.gserviceaccount.com",
            )
            .json(&payload)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        // A test plan should have been submitted to the resolver
        let res = tss.resolver_rx.try_recv();
        assert!(res.is_ok(), "{res:?}");

        // The run should be initialising
        let summary: TestRunSummary = resp.json();
        assert_eq!(summary.current_status, Status::Initialising);
        assert_eq!(
            summary.initiated_by,
            "ci@my-project.iam.gserviceaccount.com"
        );

        // The run should be in the DB, with the initiator captured from the IAP header
        let maybe_run = TestRun::get_by_uuid(&summary.id, conn!()).await.unwrap();
        let run = maybe_run.expect("test run ID did not map to a known run in the DB");
        assert_eq!(
            run.initiated_by(),
            Some("ci@my-project.iam.gserviceaccount.com")
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_records_unknown_when_the_iap_header_is_absent() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let payload = tss.minimal_trigger_payload();

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let summary: TestRunSummary = resp.json();
        assert_eq!(summary.initiated_by, "unknown");

        let tr = TestRun::get_by_uuid(&summary.id, conn!())
            .await?
            .expect("test run should be in the DB");
        assert_eq!(tr.initiated_by(), None);

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
