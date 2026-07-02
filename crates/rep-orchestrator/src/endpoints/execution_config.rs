//! Fetch RTF config sections required for running a given [TestExecution] by its UUID.
use crate::{
    Error, Result, conn,
    db::{TestExecution, TestRun},
    endpoints::BearerToken,
    state::ServerState,
};
use axum::{
    Json,
    extract::{Path, State},
};
use rep_orchestrator_shared::PrometheusQueriesResponse;
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

pub async fn prometheus_handler(
    auth: BearerToken,
    Path(id): Path<Uuid>,
    State(ServerState { .. }): State<ServerState>,
) -> Result<Json<PrometheusQueriesResponse>> {
    let conn = conn!();
    let ex = match TestExecution::get_by_uuid(&id, conn).await? {
        Some(ex) => ex,
        None => return Err(Error::Unauthorized),
    };
    auth.verify(ex.token())?;

    let payload = TestRun::cached_payload_for_run_id(ex.test_run_id(), conn)
        .await?
        .ok_or(Error::UnknownTestExecution { id })?;

    let namespace = id.to_string();
    let environment = payload
        .test_plan
        .environment
        .execution
        .output_collection
        .prometheus
        .iter()
        .map(|q| q.with_namespace_label_filter(&namespace))
        .collect();
    let scenario = payload
        .test_plan
        .scenario
        .execution
        .output_collection
        .prometheus
        .iter()
        .map(|q| q.with_namespace_label_filter(&namespace))
        .collect();

    Ok(Json(PrometheusQueriesResponse {
        environment,
        scenario,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        context::RepContext,
        db::{self, TestRun},
        test_helpers::TestServerState,
    };
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use rep_orchestrator_shared::payload::TriggerPayload;
    use reqwest::StatusCode;
    use rtf_config::formats::{OutputCollection, PrometheusQuery};
    use simple_test_case::test_case;
    use sqlx::PgConnection;

    fn bearer(token: Uuid) -> HeaderValue {
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
    }

    async fn provision(ex: TestExecution, run_uuid: Uuid, tss: &TestServerState) {
        let TriggerPayload {
            test_plan,
            relative_files,
            custom_providers,
            ..
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

    async fn cache_payload_with_output_collection(
        tr: &TestRun,
        output_collection: OutputCollection,
        conn: &mut PgConnection,
        tss: &TestServerState,
    ) -> db::Result<()> {
        let mut payload = tss.minimal_trigger_payload();
        let oc_json = serde_json::to_value(&output_collection).unwrap();
        payload["test_plan"]["scenario"]["output_collection"] = oc_json.clone();
        payload["test_plan"]["environment"]["output_collection"] = oc_json;

        let payload: TriggerPayload = serde_json::from_value(payload).unwrap();
        tr.cache_payload(&payload, conn).await
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
            let tr = TestRun::init("test", None, conn).await?;
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
    #[test_case("prometheus-queries"; "prometheus")]
    #[tokio::test]
    async fn handlers_returns_403_without_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let tr = TestRun::init("test", None, conn).await?;
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
    #[test_case("prometheus-queries"; "prometheus")]
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
    #[test_case("prometheus-queries"; "prometheus")]
    #[tokio::test]
    async fn handlers_returns_403_with_wrong_token(endpoint: &str) -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let ex_uuid = {
            let conn = conn!();
            let tr = TestRun::init("test", None, conn).await?;
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
    async fn prometheus_handler_returns_empty_lists_when_no_queries_configured()
    -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_uuid, token) = {
            let conn = conn!();
            let tr = TestRun::init("test", None, conn).await?;
            let ex = tr.init_execution("test", 0, conn).await?;
            let uuids = (ex.uuid(), ex.token());

            cache_payload_with_output_collection(&tr, OutputCollection::default(), conn, &tss)
                .await?;

            uuids
        };

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/prometheus-queries"))
            .add_header(AUTHORIZATION, bearer(token))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: PrometheusQueriesResponse = resp.json();
        assert!(body.environment.is_empty(), "{body:?}");
        assert!(body.scenario.is_empty(), "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn prometheus_handler_returns_namespace_injected_queries() -> anyhow::Result<()> {
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
            let tr = TestRun::init("test", None, conn).await?;
            let ex = tr.init_execution("test", 0, conn).await?;
            let uuids = (ex.uuid(), ex.token());

            cache_payload_with_output_collection(&tr, output_collection, conn, &tss).await?;

            uuids
        };

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/prometheus-queries"))
            .add_header(AUTHORIZATION, bearer(token))
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: PrometheusQueriesResponse = resp.json();
        let expected_query = format!(r#"sum(up{{namespace="{ex_uuid}"}})"#);

        assert_eq!(body.environment.len(), 1, "{body:?}");
        assert_eq!(body.environment[0].query, expected_query, "{body:?}");
        assert_eq!(body.scenario.len(), 1, "{body:?}");
        assert_eq!(body.scenario[0].query, expected_query, "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn prometheus_handler_returns_404_when_payload_not_cached() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let (ex_uuid, token) = {
            let conn = conn!();
            let tr = TestRun::init("test", None, conn).await?;
            let ex = tr.init_execution("test", 0, conn).await?;

            // Deliberately skip provisioning/caching entirely: the token is valid, but this
            // run's payload was never written to payload_cache.
            (ex.uuid(), ex.token())
        };

        let resp = tss
            .test_server
            .get(&format!("/test-execution/{ex_uuid}/prometheus-queries"))
            .add_header(AUTHORIZATION, bearer(token))
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }
}
