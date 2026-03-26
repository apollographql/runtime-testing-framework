use assert_fs::{
    TempDir,
    prelude::{PathChild, PathCopy},
};
use rep_orchestrator_shared::payload::TriggerPayload;
use reqwest::{Client, Response};
use rtf_cli::commands::plumbing::prepare_rep_trigger_payload;
use serde::{Serialize, de::DeserializeOwned};
use std::fmt::Display;

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
        // For the sake of tests that need to volume mount into docker containers, we place our temp
        // directories in CARGO_TARGET_TMPDIR rather than /tmp. This allows us to avoid all of the
        // "fun" of OSX /tmp symlinks and the fact that docker under OSX runs in a VM that doesn't have
        // access to paths outside of the user's homedir.
        //   See https://doc.rust-lang.org/cargo/reference/environment-variables.html
        let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        tmp.copy_from(test_plan_dir, &["**"]).unwrap();

        prepare_rep_trigger_payload(
            tmp.child("test-plan.yaml").to_str().unwrap(),
            false,
            None,
            Default::default(),
        )
        .await
    }

    pub async fn trigger_run(&self, test_plan_dir: &str) -> anyhow::Result<Response> {
        let body = self.prepare_rep_payload(test_plan_dir).await?;

        self.post("test-run/trigger", body).await
    }
}
