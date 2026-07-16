use rep_orchestrator_shared::summary::TestRunSummary;
use reqwest::{StatusCode, Url};
use thiserror::Error;
use uuid::Uuid;

/// Errors produced when communicating with the orchestrator HTTP API.
#[derive(Debug, Error)]
pub enum Error {
    #[error("orchestrator returned {status} fetching run id {id}")]
    TestRunStatus { status: StatusCode, id: Uuid },

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

pub trait Client: Send + Sync + Clone + 'static {
    fn run_summary(
        &self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<TestRunSummary>, Error>> + Send;
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
}

fn test_run_status_url(base_url: &Url, id: Uuid) -> Url {
    base_url
        .join(&format!("/test-run/{id}/status"))
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

    /// A run that started "now", so whether it polls depends only on whether its status is
    /// terminal (not on the stuck-run age guard, which is unit-tested in `view`). Its single
    /// execution carries a status history so the detail view has something to render.
    fn sample_summary(run_id: Uuid, ex_id: Uuid, status: Status) -> TestRunSummary {
        TestRunSummary {
            id: run_id,
            name: "my-test-run".to_owned(),
            current_status: status,
            started_at: Utc::now(),
            executions: vec![TestExecutionSummary {
                id: ex_id,
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
            }],
            ..Default::default()
        }
    }

    #[derive(Debug, Clone)]
    pub struct MockClient {
        test_run_summary: Option<TestRunSummary>,
        status_code: StatusCode,
    }

    impl Default for MockClient {
        fn default() -> Self {
            Self {
                test_run_summary: None,
                status_code: StatusCode::OK,
            }
        }
    }

    impl MockClient {
        pub fn with_test_run(run_id: Uuid, ex_id: Uuid, status: Status) -> Self {
            Self {
                test_run_summary: Some(sample_summary(run_id, ex_id, status)),
                ..Default::default()
            }
        }

        pub fn with_status_code(status_code: StatusCode) -> Self {
            Self {
                status_code,
                ..Default::default()
            }
        }
    }

    impl Client for MockClient {
        async fn run_summary(&self, id: Uuid) -> Result<Option<TestRunSummary>, Error> {
            match self.status_code {
                StatusCode::OK => Ok(self.test_run_summary.clone()),
                _ => Err(Error::TestRunStatus {
                    status: self.status_code,
                    id,
                }),
            }
        }
    }
}
