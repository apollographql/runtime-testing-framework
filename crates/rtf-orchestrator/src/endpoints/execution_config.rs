//! Fetch RTF config sections required for running a given [TestExecution] by its UUID.
use crate::{Error, Result, conn, db::TestExecution, endpoints::BearerToken, state::ServerState};
use axum::{
    Json,
    extract::{Path, State},
};
use rtf_orchestrator_shared::OutputCollectionResponse;
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

pub async fn output_handler(
    auth: BearerToken,
    Path(id): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
) -> Result<Json<OutputCollectionResponse>> {
    let conn = conn!();
    let ex = match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => ex,
        None => return Err(Error::Unauthorized),
    };
    auth.verify(ex.token())?;

    let payload = eq_state
        .resolve_output_collection_for_execution(&ex)
        .await?;

    Ok(Json(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        context::OrchestratorContext,
        db::{ClusterId, TestRun},
        test_helpers::TestServerState,
    };
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use reqwest::StatusCode;
    use rtf_config::formats::{OutputCollection, PrometheusQuery};
    use rtf_orchestrator_shared::payload::PreparedPayload;
    use simple_test_case::test_case;

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    fn bearer(token: Uuid) -> HeaderValue {
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
    }

    async fn provision(ex: TestExecution, run_uuid: Uuid, tss: &TestServerState) {
        provision_payload(ex, run_uuid, tss.minimal_trigger_payload(), tss).await;
    }

    async fn provision_with_output_collection(
        ex: TestExecution,
        run_uuid: Uuid,
        output_collection: OutputCollection,
        tss: &TestServerState,
    ) {
        let mut payload = tss.minimal_trigger_payload();
        let oc_json = serde_json::to_value(&output_collection).unwrap();
        payload["test_plan"]["scenario"]["output_collection"] = oc_json.clone();
        payload["test_plan"]["environment"]["output_collection"] = oc_json;

        provision_payload(ex, run_uuid, payload, tss).await;
    }

    async fn provision_payload(
        ex: TestExecution,
        run_uuid: Uuid,
        payload: serde_json::Value,
        tss: &TestServerState,
    ) {
        let PreparedPayload {
            test_plan,
            relative_files,
            custom_providers,
            ..
        } = serde_json::from_value(payload).unwrap();

        let ctx = OrchestratorContext::new_from_inlined_files(
            Config::get(),
            relative_files,
            custom_providers,
        );

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
        _ = tss
            .prov_handle
            .request_provisioning(ex, run_uuid, alpha_cluster())
            .await;
    }

    async fn populate_resolved_caches(ex: &TestExecution, tss: &TestServerState) {
        tss.prov_handle.resolve_and_cache_config(ex).await.unwrap();
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case("environment-config"; "env")]
    #[test_case("scenario-config"; "scenario")]
    #[tokio::test]
    async fn handlers_returns_200_with_valid_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_uuid, token) = {
            let conn = conn!();
            let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
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
    #[test_case("output-config"; "output")]
    #[tokio::test]
    async fn handlers_returns_403_without_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
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
    #[test_case("output-config"; "output")]
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
    #[test_case("output-config"; "output")]
    #[tokio::test]
    async fn handlers_returns_403_with_wrong_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let ex_uuid = {
            let conn = conn!();
            let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
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

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn output_handler_returns_empty_lists_when_no_queries_configured() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_uuid, token) = {
            let conn = conn!();
            let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
            let ex = tr.init_execution("test", 0, conn).await?;
            let uuids = (ex.uuid(), ex.token());

            provision_with_output_collection(
                ex.clone(),
                tr.uuid(),
                OutputCollection::default(),
                &tss,
            )
            .await;
            populate_resolved_caches(&ex, &tss).await;

            uuids
        };

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/output-config"))
            .add_header(AUTHORIZATION, bearer(token))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: OutputCollectionResponse = resp.json();
        assert!(body.prometheus.environment.is_empty(), "{body:?}");
        assert!(body.prometheus.scenario.is_empty(), "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn output_handler_returns_namespace_injected_queries() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let output_collection = OutputCollection {
            prometheus: vec![PrometheusQuery {
                name: "up_metric".to_owned(),
                step: "15s".to_owned(),
                query: "sum(up)".to_owned(),
            }],
        };

        let (ex_uuid, token) = {
            let conn = conn!();
            let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), conn).await?;
            let ex = tr.init_execution("test", 0, conn).await?;
            let uuids = (ex.uuid(), ex.token());

            provision_with_output_collection(ex.clone(), tr.uuid(), output_collection, &tss).await;
            populate_resolved_caches(&ex, &tss).await;

            uuids
        };

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/output-config"))
            .add_header(AUTHORIZATION, bearer(token))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: OutputCollectionResponse = resp.json();
        let expected_query = format!(r#"sum(up{{namespace="{ex_uuid}"}})"#);

        assert_eq!(body.prometheus.environment.len(), 1, "{body:?}");
        assert_eq!(
            body.prometheus.environment[0].query, expected_query,
            "{body:?}"
        );
        assert_eq!(body.prometheus.scenario.len(), 1, "{body:?}");
        assert_eq!(
            body.prometheus.scenario[0].query, expected_query,
            "{body:?}"
        );

        Ok(())
    }
}
