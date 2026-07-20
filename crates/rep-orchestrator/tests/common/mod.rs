use anyhow::{Context as _, anyhow, bail, ensure};
use axum::body::Bytes;
use rep_orchestrator_shared::{
    status::Status,
    summary::{TestExecutionSummary, TestRunSummary},
    {payload::TriggerPayload, test_plan::Rep},
};
use reqwest::{Client, Response};
use rtf_config::{
    context::Context,
    formats::{PrometheusQuery, TestPlan},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    env,
    fmt::Display,
    io::{self, Read},
    time::Duration,
};
use tokio::time::{Instant, sleep};
use uuid::Uuid;
use zip::ZipArchive;

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

    async fn poll_for_condition<F, T>(
        &self,
        cond: F,
        error_msg: String,
        timeout: Duration,
        interval_ms: u64,
    ) -> T
    where
        F: AsyncFn() -> Option<T>,
    {
        let deadline = Instant::now() + timeout;
        loop {
            assert!(Instant::now() < deadline, "{error_msg}");
            if let Some(t) = cond().await {
                return t;
            }

            sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    /// Poll `GET /test-run/{id}/status` until at least one execution appears, then return its UUID.
    /// Panics if no execution appears within `timeout`.
    pub async fn poll_for_execution_id(&self, run_id: Uuid, timeout: Duration) -> Uuid {
        self.poll_for_condition(
            async || {
                let summary: TestRunSummary = self
                    .json_get(format!("test-run/{run_id}/status"))
                    .await
                    .unwrap();
                summary.executions.first().map(|ex| ex.id)
            },
            format!("timed out waiting for execution to appear for run {run_id}"),
            timeout,
            500,
        )
        .await
    }

    /// Poll `GET /test-execution/{id}/status` until `expected` appears in the status history.
    /// Panics if the status does not appear within `timeout`.
    pub async fn poll_for_status(&self, ex_id: Uuid, expected: Status, timeout: Duration) {
        self.poll_for_condition(
            async || {
                let summary: TestExecutionSummary = self
                    .json_get(format!("test-execution/{ex_id}/status"))
                    .await
                    .unwrap();

                if summary.status_history.iter().any(|u| u.status == expected) {
                    Some(())
                } else {
                    None
                }
            },
            format!("timed out waiting for status {expected:?} on execution {ex_id}"),
            timeout,
            500,
        )
        .await
    }

    /// Poll `GET /test-execution/{id}/status` until `current_status` is terminal, then return
    /// the full summary. Panics if no terminal status is reached within `timeout`.
    pub async fn poll_execution_for_terminal_status(
        &self,
        ex_id: Uuid,
        timeout: Duration,
    ) -> TestExecutionSummary {
        self.poll_for_condition(
            async || {
                let summary: TestExecutionSummary = self
                    .json_get(format!("test-execution/{ex_id}/status"))
                    .await
                    .unwrap();

                if summary.current_status.is_terminal() {
                    Some(summary)
                } else {
                    None
                }
            },
            format!("timed out waiting for terminal status on execution {ex_id}"),
            timeout,
            500,
        )
        .await
    }

    pub async fn get_text(&self, endpoint: String) -> anyhow::Result<String> {
        let resp = self
            .get(&endpoint)
            .await
            .context("unable to make GET request")?;

        if !resp.status().is_success() {
            bail!("failed GET: {}", resp.status());
        }

        resp.text().await.context("unable to read body")
    }

    pub async fn get_output_zip(&self, ex_id: Uuid) -> anyhow::Result<OutputZip> {
        let resp = self
            .get(format!("test-execution/{ex_id}/output.zip"))
            .await
            .context("unable to GET output.zip")?
            .error_for_status()?;

        let bytes = resp.bytes().await.context("unable to read body")?;

        Ok(OutputZip::new(bytes))
    }

    pub fn prometheus_query_with_namespace_filter(
        &self,
        ex_id: Uuid,
        name: &str,
        raw_query: &str,
    ) -> String {
        PrometheusQuery {
            name: name.to_string(),
            step: "15s".to_string(),
            query: raw_query.to_string(),
        }
        .with_namespace_label_filter(&ex_id.to_string())
        .unwrap()
        .query
    }
}

/// A parsed `output.zip` archive, with assertion helpers for the checks integration tests
/// commonly need to make about its contents.
pub struct OutputZip {
    zip: ZipArchive<io::Cursor<Bytes>>,
    file_names: Vec<String>,
}

impl OutputZip {
    fn new(bytes: Bytes) -> Self {
        let mut zip = ZipArchive::new(io::Cursor::new(bytes))
            .context("unable to parse zip file")
            .unwrap();

        let file_names = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();

        Self { zip, file_names }
    }

    pub fn contains_path_prefix(&self, prefix: &str) -> bool {
        self.file_names.iter().any(|n| n.starts_with(prefix))
    }

    pub fn read_string(&mut self, path: &str) -> String {
        let mut buf = String::new();

        self.zip
            .by_name(path)
            .unwrap_or_else(|_| {
                panic!(
                    "{path} not found in output.zip; got: {:#?}",
                    self.file_names
                )
            })
            .read_to_string(&mut buf)
            .context(format!("unable to read {path}"))
            .unwrap();

        buf
    }

    pub fn read_json(&mut self, path: &str) -> anyhow::Result<serde_json::Value> {
        let s = self.read_string(path);
        ensure!(!s.is_empty(), "{path} exists but is an empty file");

        serde_json::from_str::<serde_json::Value>(&s).context(format!("{path} must be valid JSON"))
    }

    pub fn read_prometheus_query(&mut self, source: &str, name: &str) -> anyhow::Result<String> {
        let path = format!("output/prometheus/{source}/{name}.json");
        let s = self.read_string(&path);
        ensure!(!s.is_empty(), "{path} exists but is an empty file");

        let value: serde_json::Value =
            serde_json::from_str(&s).context(format!("{path} must be valid JSON"))?;

        value["result"][0]["metric"]["query"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("expected result[0].metric.query in {path}: {value:#?}"))
    }

    pub fn rtf_output_content(&mut self) -> String {
        let path = self
            .file_names
            .iter()
            .find(|n| n.ends_with("RTF_OUTPUT"))
            .unwrap_or_else(|| {
                panic!(
                    "no RTF_OUTPUT file in output.zip. Got: {:#?}",
                    self.file_names
                )
            })
            .clone();

        self.read_string(&path)
    }
}
