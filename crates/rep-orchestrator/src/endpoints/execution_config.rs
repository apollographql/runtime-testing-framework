//! Fetch RTF config sections required for running a given [TestExecution] by its UUID.
use crate::{Error, Result, conn, db::TestExecution, endpoints::BearerToken, state::ServerState};
use axum::extract::{Path, State};
use uuid::Uuid;

pub async fn env_handler(
    auth: BearerToken,
    Path(id): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
) -> Result<String> {
    let conn = conn!();

    let ex = match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => ex,
        None => return Err(Error::Unauthorized),
    };

    auth.verify(ex.token())?;
    let yaml = eq_state.resolve_environment_for_execution(&ex).await?;

    Ok(yaml)
}

pub async fn scenario_handler(
    auth: BearerToken,
    Path(id): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
) -> Result<String> {
    let conn = conn!();

    let ex = match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => ex,
        None => return Err(Error::Unauthorized),
    };

    auth.verify(ex.token())?;
    let yaml = eq_state.resolve_scenario_for_execution(&ex).await?;

    Ok(yaml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, context::RepContext, db::TestRun, test_helpers::TestServerState};
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use rep_orchestrator_shared::payload::TriggerPayload;
    use reqwest::StatusCode;
    use simple_test_case::test_case;

    fn bearer(token: Uuid) -> HeaderValue {
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
    }

    async fn provision(ex: TestExecution, run_uuid: Uuid, tss: &TestServerState) {
        let TriggerPayload {
            test_plan,
            relative_files,
            custom_providers,
        } = serde_json::from_value(tss.minimal_trigger_payload()).unwrap();

        let ctx = RepContext::new(Config::get(), relative_files, custom_providers);

        tss.state
            .eq_state
            .try_reserve_pending_executions(&test_plan)
            .await
            .unwrap();
        tss.prov_handle
            .cache_for_test_run(run_uuid, ctx, test_plan)
            .await;

        // Channel is closed at this point because we're not running the event loop, but we just
        // need the side effects of the push
        _ = tss.prov_handle.request_provisioning(ex, run_uuid).await;
    }

    async fn populate_resolved_caches(ex: &TestExecution, tss: &TestServerState) {
        tss.prov_handle
            .resolve_and_cache_env_config(ex)
            .await
            .unwrap();
        tss.prov_handle
            .resolve_and_cache_scenario_config(ex)
            .await
            .unwrap();
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("environment-config"; "env")]
    #[test_case("scenario-config"; "scenario")]
    #[tokio::test]
    async fn handlers_returns_200_with_valid_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_uuid, token) = {
            let conn = conn!();
            let tr = TestRun::init("test", conn).await?;
            let ex = tr.init_execution("test", 0, conn).await?;
            let uuids = (ex.uuid(), ex.token());

            provision(ex.clone(), tr.uuid(), &tss).await;
            populate_resolved_caches(&ex, &tss).await;

            uuids
        };

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/{endpoint}"))
            .add_header(AUTHORIZATION, bearer(token))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("environment-config"; "env")]
    #[test_case("scenario-config"; "scenario")]
    #[tokio::test]
    async fn handlers_returns_403_without_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", conn).await?;
        let ex_uuid = tr.init_execution("test", 0, conn).await?.uuid();

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/{endpoint}"))
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("environment-config"; "env")]
    #[test_case("scenario-config"; "scenario")]
    #[tokio::test]
    async fn handlers_returns_403_for_unknown_execution(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{}/{endpoint}", Uuid::new_v4()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("environment-config"; "env")]
    #[test_case("scenario-config"; "scenario")]
    #[tokio::test]
    async fn handlers_returns_403_with_wrong_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let ex_uuid = {
            let conn = conn!();
            let tr = TestRun::init("test", conn).await?;
            let ex = tr.init_execution("test", 0, conn).await?;
            let uuid = ex.uuid();

            provision(ex, tr.uuid(), &tss).await;

            uuid
        };

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/{endpoint}"))
            .add_header(AUTHORIZATION, bearer(Uuid::new_v4()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }
}
