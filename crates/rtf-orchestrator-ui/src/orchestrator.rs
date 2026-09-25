use cached::cached;
use chrono::{DateTime, Utc};
use reqwest::{StatusCode, Url, header::LOCATION, redirect::Policy};
use rtf_orchestrator_shared::{
    known_test_plan::{
        KnownTestPlanListParams, KnownTestPlanListResponse, KnownTestPlanRunsParams,
        KnownTestPlanSummary,
    },
    payload::TriggerPayload,
    summary::{TestExecutionSummary, TestRunListResponse, TestRunSummary},
    test_plan_details::{TestPlanDetails, TestPlanDetailsParams},
};
use thiserror::Error;
use uuid::Uuid;

/// Set by Google IAP on authenticated requests, with a value of the form `prefix:email`.
pub(crate) const IAP_USER_EMAIL_HEADER: &str = "x-goog-authenticated-user-email";

#[derive(Debug, Error)]
pub enum Error {
    #[error("orchestrator returned {status} fetching run id {id}")]
    TestRunStatus { status: StatusCode, id: Uuid },

    #[error("orchestrator returned {status} fetching execution id {id}")]
    TestExecutionStatus { status: StatusCode, id: Uuid },

    #[error("orchestrator returned {status} listing test runs")]
    ListRuns { status: StatusCode },

    #[error("orchestrator returned {status} listing known test plans")]
    ListKnownTestPlans { status: StatusCode },

    #[error("orchestrator returned {status} fetching known test plan id {uuid}")]
    KnownTestPlanStatus { status: StatusCode, uuid: Uuid },

    #[error("orchestrator returned {status} fetching details for known test plan id {uuid}")]
    TestPlanDetailsStatus { status: StatusCode, uuid: Uuid },

    #[error("orchestrator returned {status} listing test runs for known test plan id {uuid}")]
    ListKnownTestPlanRuns { status: StatusCode, uuid: Uuid },

    #[error("orchestrator returned unexpected status {status} fetching {path}")]
    Download { status: StatusCode, path: String },

    #[error("orchestrator returned {status} triggering a run: {message}")]
    Trigger { status: StatusCode, message: String },

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

/// Serialized directly as the `GET /test-run` query string, so field names must match its params.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RunListFilter {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub initiated_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_after: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone)]
pub enum Download {
    Ready(Vec<u8>),
    /// The parent run/execution doesn't exist, or the artifact was never produced.
    NotFound,
    /// The parent run/execution exists, but the artifact isn't ready yet.
    NotReady,
    /// To be relayed to the browser verbatim rather than followed (e.g. a 307 to a signed GCS URL).
    Redirect {
        status: StatusCode,
        location: String,
    },
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

    fn execution_log(&self, id: Uuid) -> impl Future<Output = Result<Download, Error>> + Send;

    fn execution_output_zip(
        &self,
        id: Uuid,
    ) -> impl Future<Output = Result<Download, Error>> + Send;

    fn list_runs(
        &self,
        filter: &RunListFilter,
    ) -> impl Future<Output = Result<TestRunListResponse, Error>> + Send;

    fn list_known_test_plans(
        &self,
        filter: &KnownTestPlanListParams,
    ) -> impl Future<Output = Result<KnownTestPlanListResponse, Error>> + Send;

    fn known_test_plan_summary(
        &self,
        uuid: Uuid,
    ) -> impl Future<Output = Result<Option<KnownTestPlanSummary>, Error>> + Send;

    /// Cached for 5 minutes by [HttpClient].
    fn test_plan_details(
        &self,
        uuid: Uuid,
        params: &TestPlanDetailsParams,
    ) -> impl Future<Output = Result<Option<TestPlanDetails>, Error>> + Send;

    fn list_known_test_plan_runs(
        &self,
        uuid: Uuid,
        filter: &KnownTestPlanRunsParams,
    ) -> impl Future<Output = Result<TestRunListResponse, Error>> + Send;

    /// `initiated_by` is the raw [IAP_USER_EMAIL_HEADER] value from the inbound request, if any.
    fn trigger(
        &self,
        payload: &TriggerPayload,
        initiated_by: Option<&str>,
    ) -> impl Future<Output = Result<TestRunSummary, Error>> + Send;
}

#[derive(Debug, Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    orchestrator_url: Url,
}

impl HttpClient {
    pub fn try_new(orchestrator_url: String) -> anyhow::Result<Self> {
        Ok(Self {
            // Redirects are relayed to the browser by `fetch_download`, not followed.
            client: reqwest::Client::builder()
                .redirect(Policy::none())
                .build()?,
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

    async fn list_known_test_plans(
        &self,
        filter: &KnownTestPlanListParams,
    ) -> Result<KnownTestPlanListResponse, Error> {
        let url = self
            .orchestrator_url
            .join("/test-plan")
            .expect("base url should be valid");
        let response = self.client.get(url).query(filter).send().await?;

        match response.status() {
            StatusCode::OK => Ok(response.json().await?),
            status => Err(Error::ListKnownTestPlans { status }),
        }
    }

    async fn known_test_plan_summary(
        &self,
        uuid: Uuid,
    ) -> Result<Option<KnownTestPlanSummary>, Error> {
        let url = known_test_plan_url(&self.orchestrator_url, uuid);
        let response = self.client.get(url).send().await?;

        match response.status() {
            StatusCode::OK => Ok(Some(response.json().await?)),
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(Error::KnownTestPlanStatus { status, uuid }),
        }
    }

    async fn test_plan_details(
        &self,
        uuid: Uuid,
        params: &TestPlanDetailsParams,
    ) -> Result<Option<TestPlanDetails>, Error> {
        fetch_test_plan_details(
            self.client.clone(),
            self.orchestrator_url.clone(),
            uuid,
            params.clone(),
        )
        .await
    }

    async fn list_known_test_plan_runs(
        &self,
        uuid: Uuid,
        filter: &KnownTestPlanRunsParams,
    ) -> Result<TestRunListResponse, Error> {
        let url = known_test_plan_runs_url(&self.orchestrator_url, uuid);
        let response = self.client.get(url).query(filter).send().await?;

        match response.status() {
            StatusCode::OK => Ok(response.json().await?),
            status => Err(Error::ListKnownTestPlanRuns { status, uuid }),
        }
    }

    async fn trigger(
        &self,
        payload: &TriggerPayload,
        initiated_by: Option<&str>,
    ) -> Result<TestRunSummary, Error> {
        let url = self
            .orchestrator_url
            .join("/test-run/trigger")
            .expect("base url should be valid");
        let mut request = self.client.post(url).json(payload);

        if let Some(initiated_by) = initiated_by {
            request = request.header(IAP_USER_EMAIL_HEADER, initiated_by);
        }
        let response = request.send().await?;
        let status = response.status();

        if status.is_success() {
            return Ok(response.json().await?);
        }

        // Non-orchestrator errors (e.g. from a proxy) won't have a `message` body.
        let message = response
            .json::<TriggerErrorBody>()
            .await
            .map(|body| body.message)
            .unwrap_or_else(|_| status.to_string());

        return Err(Error::Trigger { status, message });

        // serde structs

        #[derive(Debug, serde::Deserialize)]
        struct TriggerErrorBody {
            message: String,
        }
    }
}

// The key must include every param that changes the response.
#[cached(
    ttl_secs = 300,
    key = "String",
    convert = r#"{ format!("{uuid}-{:?}-{}-{}", params.git_ref, params.days_back, params.days) }"#
)]
async fn fetch_test_plan_details(
    client: reqwest::Client,
    orchestrator_url: Url,
    uuid: Uuid,
    params: TestPlanDetailsParams,
) -> Result<Option<TestPlanDetails>, Error> {
    let url = test_plan_details_url(&orchestrator_url, uuid);
    let response = client.get(url).query(&params).send().await?;

    match response.status() {
        StatusCode::OK => Ok(Some(response.json().await?)),
        StatusCode::NOT_FOUND => Ok(None),
        status => Err(Error::TestPlanDetailsStatus { status, uuid }),
    }
}

impl HttpClient {
    async fn fetch_download(&self, path: &str) -> Result<Download, Error> {
        let url = self
            .orchestrator_url
            .join(path)
            .expect("base url should be valid");
        let response = self.client.get(url).send().await?;
        let status = response.status();

        if status.is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
                .ok_or(Error::Download {
                    status,
                    path: path.to_owned(),
                })?;

            return Ok(Download::Redirect { status, location });
        }

        match status {
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

fn known_test_plan_url(base_url: &Url, uuid: Uuid) -> Url {
    base_url
        .join(&format!("/test-plan/{uuid}"))
        .expect("base url should be valid")
}

fn known_test_plan_runs_url(base_url: &Url, uuid: Uuid) -> Url {
    base_url
        .join(&format!("/test-plan/{uuid}/runs"))
        .expect("base url should be valid")
}

fn test_plan_details_url(base_url: &Url, uuid: Uuid) -> Url {
    base_url
        .join(&format!("/test-plan/{uuid}/details"))
        .expect("base url should be valid")
}

pub(crate) mod mocks {
    use super::*;
    use chrono::Utc;
    use rtf_config::{
        formats::{EnvironmentService, ServiceReplicas},
        templating::Scalar,
    };
    use rtf_orchestrator_shared::{
        status::{Status, StatusUpdate},
        summary::TestExecutionSummary,
        test_plan_details::{
            ComposeEnvironmentSummary, ConfigSection, EnvironmentSummary, K8sEnvironmentSummary,
            ManifestLocation, MatrixSummary, TestPlanHistory, TestPlanSource, TestPlanVariable,
            VariableDeclaration, VariableValue,
        },
    };
    use std::collections::BTreeMap;

    pub(crate) fn sample_execution(run_id: Uuid, ex_id: Uuid) -> TestExecutionSummary {
        TestExecutionSummary {
            id: ex_id,
            test_run_id: Some(run_id),
            cluster: Some("alpha".to_owned()),
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

    pub(crate) fn sample_summary_with_initiator(
        run_id: Uuid,
        ex_id: Uuid,
        status: Status,
        initiator: &str,
    ) -> TestRunSummary {
        TestRunSummary {
            id: run_id,
            name: "my-test-run".to_owned(),
            cluster: "alpha".to_owned(),
            current_status: status,
            initiated_by: initiator.to_owned(),
            started_at: Utc::now(),
            executions: vec![TestExecutionSummary {
                test_run_id: None,
                cluster: None,
                ..sample_execution(run_id, ex_id)
            }],
            ..Default::default()
        }
    }

    /// Starts at `Utc::now()`, so whether it polls depends only on `status`.
    pub(crate) fn sample_summary(run_id: Uuid, ex_id: Uuid, status: Status) -> TestRunSummary {
        sample_summary_with_initiator(run_id, ex_id, status, "someone@apollographql.com")
    }

    pub(crate) fn sample_known_test_plan(uuid: Uuid) -> KnownTestPlanSummary {
        KnownTestPlanSummary {
            uuid,
            name: "my-known-test-plan".to_owned(),
            description: Some("a sample known test plan".to_owned()),
            org: "apollographql".to_owned(),
            repo: "runtime-testing-framework".to_owned(),
            path: "test-plans/example.yaml".to_owned(),
            pinned_workload_cluster: None,
            allow_k8s_write: false,
        }
    }

    pub(crate) fn sample_test_plan_details(uuid: Uuid) -> TestPlanDetails {
        TestPlanDetails {
            uuid,
            name: "my-known-test-plan".to_owned(),
            description: Some("a sample known test plan".to_owned()),
            cluster: "alpha".to_owned(),
            source: TestPlanSource {
                org: "apollographql".to_owned(),
                repo: "runtime-testing-framework".to_owned(),
                path: "test-plans/example.yaml".to_owned(),
                git_ref: None,
                sha: "abc1234def5678".to_owned(),
            },
            variables: vec![
                TestPlanVariable {
                    name: "duration".to_owned(),
                    declarations: vec![VariableDeclaration {
                        section: ConfigSection::Scenario,
                        description: "Main test duration".to_owned(),
                        default: Some(Scalar::from("5m")),
                        allowed_values: None,
                    }],
                    current_value: Some(VariableValue::Scalar(Scalar::from("60s"))),
                    required: false,
                },
                TestPlanVariable {
                    name: "region".to_owned(),
                    declarations: vec![VariableDeclaration {
                        section: ConfigSection::Environment,
                        description: "Where to run".to_owned(),
                        default: None,
                        allowed_values: None,
                    }],
                    current_value: Some(VariableValue::Dimension(vec![
                        Scalar::from("us-east-1"),
                        Scalar::from("eu-west-1"),
                    ])),
                    required: false,
                },
                TestPlanVariable {
                    name: "tier".to_owned(),
                    declarations: vec![VariableDeclaration {
                        section: ConfigSection::Environment,
                        description: "Service tier".to_owned(),
                        default: Some(Scalar::from("free")),
                        allowed_values: Some(vec![
                            Scalar::from("free"),
                            Scalar::from("paid"),
                            Scalar::from("enterprise"),
                        ]),
                    }],
                    current_value: None,
                    required: false,
                },
            ],
            matrix: MatrixSummary {
                n_executions: 8,
                dimensions: BTreeMap::from([(
                    "region".to_owned(),
                    vec![Scalar::from("us-east-1"), Scalar::from("eu-west-1")],
                )]),
                compound_groups: BTreeMap::from([
                    (
                        "subjects".to_owned(),
                        vec![
                            BTreeMap::from([
                                ("setup_subject".to_owned(), Scalar::from("world!")),
                                ("scenario_subject".to_owned(), Scalar::from("sailor")),
                            ]),
                            BTreeMap::from([
                                ("setup_subject".to_owned(), Scalar::from("mother")),
                                ("scenario_subject".to_owned(), Scalar::from("father")),
                            ]),
                        ],
                    ),
                    (
                        "colours".to_owned(),
                        vec![
                            BTreeMap::from([
                                ("foreground".to_owned(), Scalar::from("red")),
                                ("background".to_owned(), Scalar::from("blue")),
                            ]),
                            BTreeMap::from([
                                ("foreground".to_owned(), Scalar::from("black")),
                                ("background".to_owned(), Scalar::from("white")),
                            ]),
                        ],
                    ),
                ]),
            },
            environment: Some(EnvironmentSummary::DockerCompose(
                ComposeEnvironmentSummary {
                    services: vec![
                        EnvironmentService {
                            name: "web".to_owned(),
                            image: Some("nginx:1.25".to_owned()),
                            replicas: ServiceReplicas::Fixed(1),
                        },
                        EnvironmentService {
                            name: "worker".to_owned(),
                            image: Some("my-worker:latest".to_owned()),
                            replicas: ServiceReplicas::Variable("${WORKER_REPLICAS}".to_owned()),
                        },
                    ],
                    has_variable_replicas: true,
                    services_vary_by_matrix: false,
                    resolved_for_variant: Some("region_us-east-1".to_owned()),
                },
            )),
            history: TestPlanHistory::default(),
        }
    }

    pub(crate) fn sample_k8s_test_plan_details(uuid: Uuid) -> TestPlanDetails {
        TestPlanDetails {
            environment: Some(EnvironmentSummary::K8s(K8sEnvironmentSummary {
                manifests: vec![
                    ManifestLocation::gh(
                        "deployment.yaml",
                        "https://github.com/apollographql/runtime-testing-framework/blob/main/k8s/deployment.yaml",
                    ),
                    ManifestLocation::gh(
                        "service.yaml",
                        "https://github.com/apollographql/runtime-testing-framework/blob/main/k8s/service.yaml",
                    ),
                    ManifestLocation::inline(
                        "kustomization.yaml",
                        "https://github.com/apollographql/runtime-testing-framework/blob/main/test-plans/example.yaml",
                    ),
                ],
                resolved_for_variant: Some("region_us-east-1".to_owned()),
            })),
            ..sample_test_plan_details(uuid)
        }
    }

    /// Always succeeds with canned data.
    #[cfg(test)]
    #[derive(Debug, Clone)]
    pub struct MockClient {
        test_run_summary: TestRunSummary,
        test_execution_summary: TestExecutionSummary,
    }

    #[cfg(test)]
    impl MockClient {
        pub fn with_test_run(run_id: Uuid, ex_id: Uuid, status: Status) -> Self {
            Self {
                test_run_summary: sample_summary(run_id, ex_id, status),
                test_execution_summary: sample_execution(run_id, ex_id),
            }
        }
    }

    #[cfg(test)]
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

        async fn list_known_test_plans(
            &self,
            _filter: &KnownTestPlanListParams,
        ) -> Result<KnownTestPlanListResponse, Error> {
            Ok(KnownTestPlanListResponse {
                test_plans: vec![sample_known_test_plan(Uuid::new_v4())],
                total: 1,
            })
        }

        async fn known_test_plan_summary(
            &self,
            uuid: Uuid,
        ) -> Result<Option<KnownTestPlanSummary>, Error> {
            Ok(Some(sample_known_test_plan(uuid)))
        }

        async fn test_plan_details(
            &self,
            uuid: Uuid,
            _params: &TestPlanDetailsParams,
        ) -> Result<Option<TestPlanDetails>, Error> {
            Ok(Some(sample_test_plan_details(uuid)))
        }

        async fn list_known_test_plan_runs(
            &self,
            _uuid: Uuid,
            _filter: &KnownTestPlanRunsParams,
        ) -> Result<TestRunListResponse, Error> {
            Ok(TestRunListResponse {
                runs: vec![self.test_run_summary.clone()],
                total: 1,
            })
        }

        async fn trigger(
            &self,
            _payload: &TriggerPayload,
            _initiated_by: Option<&str>,
        ) -> Result<TestRunSummary, Error> {
            Ok(self.test_run_summary.clone())
        }
    }
}
