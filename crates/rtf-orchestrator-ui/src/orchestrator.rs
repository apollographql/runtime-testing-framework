use chrono::{DateTime, Utc};
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunListResponse, TestRunSummary};
use reqwest::{StatusCode, Url};
use thiserror::Error;
use uuid::Uuid;

/// Errors produced when communicating with the orchestrator HTTP API.
#[derive(Debug, Error)]
pub enum Error {
    #[error("orchestrator returned {status} fetching run id {id}")]
    TestRunStatus { status: StatusCode, id: Uuid },

    #[error("orchestrator returned {status} fetching execution id {id}")]
    TestExecutionStatus { status: StatusCode, id: Uuid },

    #[error("orchestrator returned {status} listing test runs")]
    ListRuns { status: StatusCode },

    #[error("orchestrator returned unexpected status {status} fetching {path}")]
    Download { status: StatusCode, path: String },

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

/// Query filters accepted by the orchestrator's `GET /test-run` endpoint. Field names match the
/// orchestrator's query params exactly, since this is serialized directly as the request's query
/// string.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RunListFilter {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub initiated_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_after: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

/// Outcome of proxying a file download from the orchestrator.
#[derive(Debug, Clone)]
pub enum Download {
    /// The artifact exists and was fetched successfully.
    Ready(Vec<u8>),
    /// The parent run/execution doesn't exist, or the artifact was never produced.
    NotFound,
    /// The parent run/execution exists, but the artifact isn't ready to download yet.
    NotReady,
}

pub trait Client: Send + Sync + Clone + 'static {
    fn run_summary(
        &self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<TestRunSummary>, Error>> + Send;

    fn execution_summary(
        &self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<TestExecutionSummary>, Error>> + Send;

    /// Fetch a single execution's log file.
    fn execution_log(&self, id: Uuid) -> impl Future<Output = Result<Download, Error>> + Send;

    /// Fetch a single execution's zipped output directory.
    fn execution_output_zip(
        &self,
        id: Uuid,
    ) -> impl Future<Output = Result<Download, Error>> + Send;

    /// List historic test runs matching `filter`, newest-first.
    fn list_runs(
        &self,
        filter: &RunListFilter,
    ) -> impl Future<Output = Result<TestRunListResponse, Error>> + Send;
}

#[derive(Debug, Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    orchestrator_url: Url,
}

impl HttpClient {
    /// Creates a new [HttpClient] that will report updates to `orchestrator_url` for the test execution associated
    /// with `execution_id`, authenticating requests with `execution_token` as a Bearer token.
    pub fn try_new(orchestrator_url: String) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::new(),
            orchestrator_url: Url::parse(&orchestrator_url)?,
        })
    }
}

impl Client for HttpClient {
    async fn run_summary(&self, id: Uuid) -> Result<Option<TestRunSummary>, Error> {
        let url = test_run_status_url(&self.orchestrator_url, id);
        let response = self.client.get(url).send().await?;

        match response.status() {
            StatusCode::OK => Ok(Some(response.json().await?)),
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(Error::TestRunStatus { status, id }),
        }
    }

    async fn execution_summary(&self, id: Uuid) -> Result<Option<TestExecutionSummary>, Error> {
        let url = test_execution_status_url(&self.orchestrator_url, id);
        let response = self.client.get(url).send().await?;

        match response.status() {
            StatusCode::OK => Ok(Some(response.json().await?)),
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(Error::TestExecutionStatus { status, id }),
        }
    }

    async fn execution_log(&self, id: Uuid) -> Result<Download, Error> {
        let path = format!("/test-execution/{id}/log.txt");

        self.fetch_download(&path).await
    }

    async fn execution_output_zip(&self, id: Uuid) -> Result<Download, Error> {
        let path = format!("/test-execution/{id}/output.zip");

        self.fetch_download(&path).await
    }

    async fn list_runs(&self, filter: &RunListFilter) -> Result<TestRunListResponse, Error> {
        let url = self
            .orchestrator_url
            .join("/test-run")
            .expect("base url should be valid");
        let response = self.client.get(url).query(filter).send().await?;

        match response.status() {
            StatusCode::OK => Ok(response.json().await?),
            status => Err(Error::ListRuns { status }),
        }
    }
}

impl HttpClient {
    /// Fetch `path` from the orchestrator, mapping its response into a [`Download`] outcome.
    /// `reqwest` follows redirects by default, so this transparently handles endpoints that
    /// 307-redirect to a signed GCS URL as well as ones that stream bytes directly.
    async fn fetch_download(&self, path: &str) -> Result<Download, Error> {
        let url = self
            .orchestrator_url
            .join(path)
            .expect("base url should be valid");
        let response = self.client.get(url).send().await?;

        match response.status() {
            StatusCode::OK => Ok(Download::Ready(response.bytes().await?.to_vec())),
            StatusCode::NOT_FOUND => Ok(Download::NotFound),
            StatusCode::CONFLICT => Ok(Download::NotReady),
            status => Err(Error::Download {
                status,
                path: path.to_owned(),
            }),
        }
    }
}

fn test_run_status_url(base_url: &Url, id: Uuid) -> Url {
    base_url
        .join(&format!("/test-run/{id}/status"))
        .expect("base url should be valid")
}

fn test_execution_status_url(base_url: &Url, id: Uuid) -> Url {
    base_url
        .join(&format!("/test-execution/{id}/status"))
        .expect("base url should be valid")
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use chrono::Utc;
    use rep_orchestrator_shared::{
        status::{Status, StatusUpdate},
        summary::TestExecutionSummary,
    };

    /// A single execution, carrying a status history so the detail view has something to render.
    pub(crate) fn sample_execution(run_id: Uuid, ex_id: Uuid) -> TestExecutionSummary {
        TestExecutionSummary {
            id: ex_id,
            test_run_id: Some(run_id),
            name: "exec-alpha".to_owned(),
            current_status: Status::Successful,
            exit_code: Some(0),
            status_history: vec![
                StatusUpdate {
                    status: Status::Successful,
                    message: Some("execution finished".to_owned()),
                    updated_at: Utc::now(),
                },
                StatusUpdate {
                    status: Status::Running,
                    message: None,
                    updated_at: Utc::now(),
                },
            ],
            ..Default::default()
        }
    }

    /// A run that started "now", so whether it polls depends only on whether its status is
    /// terminal (not on the stuck-run age guard, which is unit-tested in `view`). Its single
    /// execution carries a status history so the detail view has something to render, but (as with
    /// the real orchestrator) does not carry a `test_run_id`.
    pub(crate) fn sample_summary(run_id: Uuid, ex_id: Uuid, status: Status) -> TestRunSummary {
        TestRunSummary {
            id: run_id,
            name: "my-test-run".to_owned(),
            current_status: status,
            initiated_by: "someone@apollographql.com".to_owned(),
            started_at: Utc::now(),
            executions: vec![TestExecutionSummary {
                test_run_id: None,
                ..sample_execution(run_id, ex_id)
            }],
            ..Default::default()
        }
    }

    /// A client that always succeeds with a canned run/execution summary. Since the not-found and
    /// error branches are now covered directly by `endpoints::run_status_body` and
    /// `execution_detail_body` (no client involved), this only needs to prove that a handler calls
    /// its client and renders whatever comes back.
    #[derive(Debug, Clone)]
    pub struct MockClient {
        test_run_summary: TestRunSummary,
        test_execution_summary: TestExecutionSummary,
    }

    impl MockClient {
        pub fn with_test_run(run_id: Uuid, ex_id: Uuid, status: Status) -> Self {
            Self {
                test_run_summary: sample_summary(run_id, ex_id, status),
                test_execution_summary: sample_execution(run_id, ex_id),
            }
        }
    }

    impl Client for MockClient {
        async fn run_summary(&self, _id: Uuid) -> Result<Option<TestRunSummary>, Error> {
            Ok(Some(self.test_run_summary.clone()))
        }

        async fn execution_summary(
            &self,
            _id: Uuid,
        ) -> Result<Option<TestExecutionSummary>, Error> {
            Ok(Some(self.test_execution_summary.clone()))
        }

        async fn execution_log(&self, _id: Uuid) -> Result<Download, Error> {
            Ok(Download::Ready(b"log contents".to_vec()))
        }

        async fn execution_output_zip(&self, _id: Uuid) -> Result<Download, Error> {
            Ok(Download::Ready(b"output zip contents".to_vec()))
        }

        async fn list_runs(&self, _filter: &RunListFilter) -> Result<TestRunListResponse, Error> {
            Ok(TestRunListResponse {
                runs: vec![self.test_run_summary.clone()],
                total: 1,
            })
        }
    }
}
