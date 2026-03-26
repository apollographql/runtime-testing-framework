use rep_orchestrator_shared::{payload::TriggerPayload, test_plan::Rep};
use reqwest::{Client, Response};
use rtf_config::{context::Context, formats::TestPlan};
use serde::{Serialize, de::DeserializeOwned};
use std::{env, fmt::Display};

const SERVER_URL: &str = "http://localhost:8035";

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
}
