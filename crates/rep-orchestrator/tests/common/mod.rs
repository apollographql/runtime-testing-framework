use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
    Api, Config as KubeConfig,
    config::{KubeConfigOptions, Kubeconfig},
};
use rep_orchestrator_shared::{
    status::Status,
    summary::{TestExecutionSummary, TestRunSummary},
    {payload::TriggerPayload, test_plan::Rep},
};
use reqwest::{Client, Response};
use rtf_config::{context::Context, formats::TestPlan};
use serde::{Serialize, de::DeserializeOwned};
use std::{env, fmt::Display, path::Path, time::Duration};
use tokio::time::sleep;
use uuid::Uuid;

const SERVER_URL: &str = "http://localhost:8035";

#[macro_export]
macro_rules! assert_status {
    ($resp:expr, $expected:expr) => {{
        let status = $resp.status();
        let body = $resp.text().await.unwrap_or_default();
        assert_eq!(status, $expected, "body: {body}");
    }};
}

pub struct TestHelper {
    client: Client,
}

impl TestHelper {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn get(&self, endpoint: impl Display) -> anyhow::Result<Response> {
        Ok(self
            .client
            .get(format!("{SERVER_URL}/{endpoint}"))
            .send()
            .await?)
    }

    pub async fn json_get<T>(&self, endpoint: impl Display) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        Ok(self.get(endpoint).await?.json().await?)
    }

    pub async fn post<T>(&self, endpoint: impl Display, body: T) -> anyhow::Result<Response>
    where
        T: Serialize,
    {
        Ok(self
            .client
            .post(format!("{SERVER_URL}/{endpoint}"))
            .body(serde_json::to_vec(&body)?)
            .header("Content-Type", "application/json")
            .send()
            .await?)
    }

    pub async fn json_post<T, U>(&self, endpoint: impl Display, body: T) -> anyhow::Result<U>
    where
        T: Serialize,
        U: DeserializeOwned,
    {
        Ok(self.post(endpoint, body).await?.json().await?)
    }

    pub async fn prepare_rep_payload(&self, test_plan_dir: &str) -> anyhow::Result<TriggerPayload> {
        let ctx = Context::new_from_env_vars(&env::vars().collect());
        let (test_plan, sources) = TestPlan::<Rep>::try_load_and_resolve_from_path(
            format!("{test_plan_dir}/test-plan.yaml"),
            &ctx,
        )
        .await?;

        TriggerPayload::prepare(test_plan, sources, Default::default(), ctx).await
    }

    pub async fn trigger_run(&self, test_plan_dir: &str) -> anyhow::Result<Response> {
        let body = self.prepare_rep_payload(test_plan_dir).await?;

        self.post("test-run/trigger", body).await
    }

    /// Poll `GET /test-run/{id}/status` until at least one execution appears, then return its UUID.
    /// Panics if no execution appears within `timeout`.
    pub async fn poll_for_execution_id(&self, run_id: Uuid, timeout: Duration) -> Uuid {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for execution to appear for run {run_id}"
            );
            let summary: TestRunSummary = self
                .json_get(format!("test-run/{run_id}/status"))
                .await
                .unwrap();
            if let Some(ex) = summary.executions.first() {
                return ex.id;
            }
            sleep(Duration::from_millis(500)).await;
        }
    }

    /// Poll `GET /test-execution/{id}/status` until `expected` appears in the status history.
    /// Panics if the status does not appear within `timeout`.
    pub async fn poll_for_status(&self, ex_id: Uuid, expected: Status, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for status {expected:?} on execution {ex_id}"
            );
            let summary: TestExecutionSummary = self
                .json_get(format!("test-execution/{ex_id}/status"))
                .await
                .unwrap();
            if summary.status_history.iter().any(|u| u.status == expected) {
                return;
            }
            sleep(Duration::from_millis(500)).await;
        }
    }
}

/// Assert that a ConfigMap with the given name exists in the `cluster-api` namespace of the
/// management cluster. Reads `RTF_KUBECONFIG_PATH` and `RTF_MGMT_CONTEXT` from the environment.
pub async fn assert_configmap_exists(name: &str) {
    let kubeconfig_path = env::var("RTF_KUBECONFIG_PATH").expect("RTF_KUBECONFIG_PATH must be set");
    let mgmt_context = env::var("RTF_MGMT_CONTEXT").expect("RTF_MGMT_CONTEXT must be set");

    let kfg = Kubeconfig::read_from(Path::new(&kubeconfig_path)).unwrap();
    let cfg = KubeConfig::from_custom_kubeconfig(
        kfg,
        &KubeConfigOptions {
            context: Some(mgmt_context),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let client = kube::Client::try_from(cfg).unwrap();
    let api: Api<ConfigMap> = Api::namespaced(client, "cluster-api");

    api.get(name)
        .await
        .unwrap_or_else(|e| panic!("ConfigMap '{name}' not found in cluster-api namespace: {e}"));
}
