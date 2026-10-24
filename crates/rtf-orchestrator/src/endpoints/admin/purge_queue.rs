//! Queue management for cancelling in-flight runs and executions, purging any queued state.
//!
//! Admins can cancel anything, other users can only cancel runs (and their executions) that they
//! initiated or that belong to a known test plan they own. See [TestRun::check_cancel_permitted].
use crate::{
    Error, Result, conn,
    db::{TestExecution, TestRun},
    state::{KnownUser, ServerState},
};
use axum::{Json, extract::State};
use rtf_orchestrator_shared::{
    PurgeRequest,
    summary::{TestExecutionSummary, TestRunSummary},
};
use tracing::warn;

pub async fn ex_handler(
    user: KnownUser,
    State(ServerState { eq_state, .. }): State<ServerState>,
    Json(PurgeRequest { id: ex_id }): Json<PurgeRequest>,
) -> Result<Json<TestExecutionSummary>> {
    let conn = conn!();
    let ex = TestExecution::get_by_uuid(&ex_id, conn)
        .await?
        .ok_or(Error::UnknownTestExecution { id: ex_id })?;

    ex.test_run(conn)
        .await?
        .check_cancel_permitted(&user, conn)
        .await?;

    warn!(%ex_id, email = user.email(), "marking execution as cancelled and purging queue state");
    eq_state
        .purge_execution(ex.clone(), user.email(), conn)
        .await?;

    Ok(Json(ex.try_into_summary_with_status_history(conn).await?))
}

pub async fn run_handler(
    user: KnownUser,
    State(ServerState { eq_state, .. }): State<ServerState>,
    Json(PurgeRequest { id: run_id }): Json<PurgeRequest>,
) -> Result<Json<TestRunSummary>> {
    let conn = conn!();
    let tr = TestRun::get_by_uuid(&run_id, conn)
        .await?
        .ok_or(Error::UnknownTestRun { id: run_id })?;

    tr.check_cancel_permitted(&user, conn).await?;

    warn!(%run_id, email = user.email(), "marking run as cancelled and purging queue state");
    eq_state.purge_run(tr.clone(), user.email(), conn).await?;

    Ok(Json(tr.try_into_summary_with_executions(conn).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, DEFAULT_POOL, DEFAULT_POOL_NAME},
        context::OrchestratorContext,
        db::{KnownTestPlan, KnownTestPlanRun, Queryable, Status as DbStatus, StatusTracked},
        event_loop::tests::stub_test_plan,
        iap_identity::IAP_USER_EMAIL_HEADER,
        test_helpers::TestServerState,
    };
    use axum_test::TestResponse;
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::{
        known_test_plan::{Role, UpdateKnownTestPlanRequest},
        payload::SourceKeyedArrayMap,
        status::Status,
        workload_config::WorkloadConfig,
    };
    use simple_test_case::test_case;
    use std::collections::{BTreeMap, HashSet};
    use uuid::Uuid;

    const ADMIN_EMAIL: &str = "admin@test.com";
    const OWNER_EMAIL: &str = "owner@test.com";
    const INITIATOR_EMAIL: &str = "initiator@test.com";
    const OTHER_EMAIL: &str = "other@test.com";
    const AUTOMATION_EMAIL: &str = "automation@test.com";

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn test_server() -> TestServerState {
        let mut cfg = Config::get().clone();
        cfg.automation_users = vec![AUTOMATION_EMAIL.to_owned()];

        TestServerState::new_with_config_and_admins(&cfg, &[ADMIN_EMAIL])
    }

    async fn seed_run(tss: &TestServerState, in_flight: bool) -> Result<(TestRun, TestExecution)> {
        let conn = conn!();
        let plan =
            KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), conn)
                .await?
                .validate_and_update(
                    &KnownUser::Admin(ADMIN_EMAIL.into()),
                    UpdateKnownTestPlanRequest {
                        associated_users: Some(BTreeMap::from([(
                            OWNER_EMAIL.to_owned(),
                            Some(Role::Owner),
                        )])),
                        ..Default::default()
                    },
                    &HashSet::from([DEFAULT_POOL_NAME]),
                    &HashSet::from([DEFAULT_POOL_NAME]),
                    conn,
                )
                .await?;

        let tr = TestRun::init(
            "test",
            None,
            Some(INITIATOR_EMAIL),
            &DEFAULT_POOL,
            &WorkloadConfig::default(),
            conn,
        )
        .await?;
        KnownTestPlanRun::link(plan.id(), tr.id(), None, conn).await?;
        let ex = tr.init_execution("test", 0, conn).await?;

        if in_flight {
            let ctx = OrchestratorContext::new_from_inlined_files(
                &Config::for_test(),
                SourceKeyedArrayMap {
                    keys: vec![],
                    data: vec![],
                },
                SourceKeyedArrayMap {
                    keys: vec![],
                    data: vec![],
                },
                Default::default(),
            );

            tss.prov_handle
                .cache_for_test_run(
                    tr.uuid(),
                    None,
                    Default::default(),
                    false,
                    ctx,
                    stub_test_plan(),
                )
                .await;

            tss.prov_handle
                .register_existing_execution(ex.uuid(), tr.uuid())
                .await;
        }

        Ok((tr, ex))
    }

    async fn post_as(
        tss: &TestServerState,
        path: &str,
        id: Uuid,
        email: Option<&str>,
    ) -> TestResponse {
        let req = tss.test_server.post(path).json(&PurgeRequest { id });
        match email {
            Some(email) => {
                req.add_header(
                    IAP_USER_EMAIL_HEADER,
                    format!("accounts.google.com:{email}"),
                )
                .await
            }
            None => req.await,
        }
    }

    #[test_case(ADMIN_EMAIL; "admin")]
    #[test_case(OWNER_EMAIL; "owner")]
    #[test_case(INITIATOR_EMAIL; "initiator")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn run_handler_permitted_user_can_cancel(email: &str) -> Result<()> {
        let tss = test_server();
        let (tr, _) = seed_run(&tss, true).await?;

        let resp = post_as(&tss, "/admin/purge-run", tr.uuid(), Some(email)).await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let summary: TestRunSummary = resp.json();
        assert_eq!(summary.current_status, Status::Cancelled);
        assert_eq!(
            summary.status_history.first().unwrap().message.as_deref(),
            Some(format!("cancelled by {email}").as_str())
        );

        Ok(())
    }

    #[test_case(Some(OTHER_EMAIL); "unrelated user")]
    #[test_case(Some(AUTOMATION_EMAIL); "automation user")]
    #[test_case(None; "unauthenticated")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn run_handler_other_callers_are_rejected(email: Option<&str>) -> Result<()> {
        let tss = test_server();
        let (tr, _) = seed_run(&tss, true).await?;

        let resp = post_as(&tss, "/admin/purge-run", tr.uuid(), email).await;
        assert_eq!(
            resp.status_code(),
            StatusCode::FORBIDDEN,
            "{:?}",
            resp.text()
        );

        let status = tr.try_current_status(conn!()).await?.unwrap().status;
        assert_ne!(status, DbStatus::Cancelled);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn run_handler_run_not_in_flight_is_rejected() -> Result<()> {
        let tss = test_server();
        let (tr, _) = seed_run(&tss, false).await?;

        let resp = post_as(&tss, "/admin/purge-run", tr.uuid(), Some(ADMIN_EMAIL)).await;
        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "{:?}",
            resp.text()
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn run_handler_unknown_run_returns_404() -> Result<()> {
        let tss = test_server();

        let resp = post_as(&tss, "/admin/purge-run", Uuid::new_v4(), Some(ADMIN_EMAIL)).await;
        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }

    #[test_case(ADMIN_EMAIL; "admin")]
    #[test_case(OWNER_EMAIL; "owner")]
    #[test_case(INITIATOR_EMAIL; "initiator")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn ex_handler_permitted_user_can_cancel(email: &str) -> Result<()> {
        let tss = test_server();
        let (_, ex) = seed_run(&tss, true).await?;

        let resp = post_as(&tss, "/admin/purge-execution", ex.uuid(), Some(email)).await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let summary: TestExecutionSummary = resp.json();
        assert_eq!(summary.current_status, Status::Cancelled);
        assert_eq!(
            summary.status_history.first().unwrap().message.as_deref(),
            Some(format!("cancelled by {email}").as_str())
        );

        Ok(())
    }

    #[test_case(Some(OTHER_EMAIL); "unrelated user")]
    #[test_case(Some(AUTOMATION_EMAIL); "automation user")]
    #[test_case(None; "unauthenticated")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn ex_handler_other_callers_are_rejected(email: Option<&str>) -> Result<()> {
        let tss = test_server();
        let (_, ex) = seed_run(&tss, true).await?;

        let resp = post_as(&tss, "/admin/purge-execution", ex.uuid(), email).await;
        assert_eq!(
            resp.status_code(),
            StatusCode::FORBIDDEN,
            "{:?}",
            resp.text()
        );

        let status = ex.try_current_status(conn!()).await?.unwrap().status;
        assert_ne!(status, DbStatus::Cancelled);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn ex_handler_execution_not_in_flight_is_rejected() -> Result<()> {
        let tss = test_server();
        let (_, ex) = seed_run(&tss, false).await?;

        let resp = post_as(&tss, "/admin/purge-execution", ex.uuid(), Some(ADMIN_EMAIL)).await;
        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "{:?}",
            resp.text()
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn ex_handler_unknown_execution_returns_404() -> Result<()> {
        let tss = test_server();

        let resp = post_as(
            &tss,
            "/admin/purge-execution",
            Uuid::new_v4(),
            Some(ADMIN_EMAIL),
        )
        .await;
        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }
}
