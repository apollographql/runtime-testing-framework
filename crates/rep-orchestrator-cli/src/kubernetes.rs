use chrono::{DateTime, Utc};
use k8s_openapi::{
    api::{
        apps::v1::Deployment,
        batch::v1::{Job, JobStatus},
        core::v1::{Event, Namespace, Pod, ServiceAccount},
    },
    apimachinery::pkg::apis::meta::v1::Time,
};
use kube::{
    Api, Config,
    api::{ListParams, LogParams, ObjectMeta, Patch, PatchParams, Request},
    config::{KubeConfigOptions, Kubeconfig, KubeconfigError},
};
use rtf_orchestrator_shared::{LOG_COLLECTION_LABEL, SCENARIO_JOB_NAME};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tokio::fs;
use tracing::warn;

const MANAGER_NAME: &str = "rep-orchestrator-cli";
const SA_NAME: &str = "results-writer";
const SA_PREFIX: &str = "iam.gke.io/gcp-service-account";
const WIF_ANNOTATION: &str = "results-writer@runtime-testing-framework.iam.gserviceaccount.com";

/// Errors produced when communicating with the Kubernetes API or reading kubeconfig.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read kubeconfig from {path}: {source}")]
    ReadKubeconfig {
        path: PathBuf,
        #[source]
        source: KubeconfigError,
    },

    #[error("failed to build kube config: {0}")]
    BuildConfig(#[source] KubeconfigError),

    #[error("failed to create kubernetes client: {0}")]
    CreateClient(#[source] kube::Error),

    #[error("failed to create namespace: {0}")]
    CreateNamespace(#[source] kube::Error),

    #[error("failed to create results-writer service account: {0}")]
    CreateServiceAccount(#[source] kube::Error),

    #[error("failed to list deployments in namespace: {0}")]
    ListDeployments(#[source] kube::Error),

    #[error("timed out after {timeout_secs}s waiting for deployments: {names}")]
    DeploymentTimeout { timeout_secs: u64, names: String },

    #[error("failed to read scenario job {name}: {source}")]
    GetScenarioJob {
        name: &'static str,
        #[source]
        source: kube::Error,
    },

    #[error("scenario job {name} has no status.startTime")]
    MissingJobStartTime { name: &'static str },
}

pub struct DeploymentStatus {
    pub total: usize,
    pub not_ready: Vec<DeploymentInfo>,
}

pub struct DeploymentInfo {
    pub name: String,
    pub last_condition_status: String,
}

pub trait Client: Send + Sync + Clone {
    fn create_namespace(&self, name: &str) -> impl Future<Output = Result<(), Error>> + Send;

    fn create_results_writer_service_account(
        &self,
        namespace: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    fn check_deployment_status(
        &self,
        namespace: &str,
    ) -> impl Future<Output = Result<DeploymentStatus, Error>> + Send;

    fn collect_container_logs(
        &self,
        namespace: &str,
        output_dir: &Path,
    ) -> impl Future<Output = ()> + Send;

    fn collect_namespace_events(
        &self,
        namespace: &str,
        output_dir: &Path,
    ) -> impl Future<Output = ()> + Send;

    fn collect_resource_metrics(
        &self,
        namespace: &str,
        output_dir: &Path,
    ) -> impl Future<Output = ()> + Send;

    fn get_scenario_job_window(
        &self,
        namespace: &str,
    ) -> impl Future<Output = Result<(DateTime<Utc>, DateTime<Utc>), Error>> + Send;
}

#[derive(Clone)]
pub struct HttpClient {
    client: kube::Client,
}

impl HttpClient {
    pub async fn from_kubeconfig(kubeconfig_path: Option<&Path>) -> crate::Result<Self> {
        let client = match kubeconfig_path {
            Some(path) => {
                let kfg = Kubeconfig::read_from(path).map_err(|source| Error::ReadKubeconfig {
                    path: path.to_owned(),
                    source,
                })?;

                let config = Config::from_custom_kubeconfig(kfg, &KubeConfigOptions::default())
                    .await
                    .map_err(Error::BuildConfig)?;

                kube::Client::try_from(config)
            }
            None => kube::Client::try_default().await,
        }
        .map_err(Error::CreateClient)?;

        Ok(Self { client })
    }
}

impl Client for HttpClient {
    async fn create_namespace(&self, name: &str) -> Result<(), Error> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        let ns = Namespace {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        api.patch(name, &PatchParams::apply(MANAGER_NAME), &Patch::Apply(&ns))
            .await
            .map_err(Error::CreateNamespace)?;

        Ok(())
    }

    async fn create_results_writer_service_account(&self, namespace: &str) -> Result<(), Error> {
        let sa = ServiceAccount {
            metadata: ObjectMeta {
                name: Some(SA_NAME.to_owned()),
                namespace: Some(namespace.to_owned()),
                annotations: Some(BTreeMap::from([(
                    SA_PREFIX.to_owned(),
                    WIF_ANNOTATION.to_owned(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };

        let api: Api<ServiceAccount> = Api::namespaced(self.client.clone(), namespace);
        api.patch(
            SA_NAME,
            &PatchParams::apply(MANAGER_NAME),
            &Patch::Apply(&sa),
        )
        .await
        .map_err(Error::CreateServiceAccount)?;

        Ok(())
    }

    async fn check_deployment_status(&self, namespace: &str) -> Result<DeploymentStatus, Error> {
        let api: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
        let deployments = api
            .list(&Default::default())
            .await
            .map_err(Error::ListDeployments)?;

        let not_ready: Vec<DeploymentInfo> = deployments
            .items
            .iter()
            .filter(|d| {
                !d.status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .is_some_and(|conditions| {
                        conditions
                            .iter()
                            .any(|c| c.type_ == "Available" && c.status == "True")
                    })
            })
            .filter_map(|d| {
                d.metadata.name.as_ref().map(|name| DeploymentInfo {
                    name: name.clone(),
                    last_condition_status: d
                        .status
                        .as_ref()
                        .and_then(|s| s.conditions.as_ref())
                        .and_then(|c| c.last())
                        .map(|c| c.status.clone())
                        .unwrap_or_else(|| "unknown".to_owned()),
                })
            })
            .collect();

        Ok(DeploymentStatus {
            total: deployments.items.len(),
            not_ready,
        })
    }

    async fn collect_container_logs(&self, namespace: &str, output_dir: &Path) {
        let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);

        let params = ListParams::default().labels(&format!("{LOG_COLLECTION_LABEL}=true"));
        let pods = match pod_api.list(&params).await {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("failed to list pods for log collection: {e}");
                warn!("{msg}");

                let errors_path = output_dir.join("logs").join("errors.txt");
                if let Some(parent) = errors_path.parent()
                    && fs::create_dir_all(parent).await.is_ok()
                {
                    let _ = fs::write(&errors_path, msg.as_bytes()).await;
                }

                return;
            }
        };

        for (pod_name, containers) in pods_to_collect(&pods.items) {
            for container_name in containers {
                let params = LogParams {
                    container: Some(container_name.clone()),
                    tail_lines: Some(10_000),
                    ..Default::default()
                };

                match pod_api.logs(pod_name, &params).await {
                    Ok(logs) => {
                        let log_path = output_dir
                            .join("logs")
                            .join(pod_name)
                            .join(format!("{container_name}.txt"));

                        if let Some(parent) = log_path.parent()
                            && let Err(e) = fs::create_dir_all(parent).await
                        {
                            warn!("failed to create log dir {}: {e}", parent.display());
                            continue;
                        }

                        if let Err(e) = fs::write(&log_path, logs.as_bytes()).await {
                            warn!("failed to write logs for {pod_name}/{container_name}: {e}");
                        }
                    }
                    Err(e) => {
                        let msg =
                            format!("failed to fetch logs for {pod_name}/{container_name}: {e}");
                        warn!("{msg}");

                        let log_path = output_dir
                            .join("logs")
                            .join(pod_name)
                            .join(format!("{container_name}.txt"));
                        if let Some(parent) = log_path.parent()
                            && fs::create_dir_all(parent).await.is_ok()
                        {
                            let _ = fs::write(&log_path, msg.as_bytes()).await;
                        }
                    }
                }
            }
        }
    }

    async fn collect_namespace_events(&self, namespace: &str, output_dir: &Path) {
        let event_api: Api<Event> = Api::namespaced(self.client.clone(), namespace);
        let mut errors = Vec::new();

        let events_val = match event_api.list(&Default::default()).await {
            Ok(events) => match serde_json::to_value(events) {
                Ok(v) => Some(v),
                Err(e) => {
                    let msg = format!("failed to serialize events: {e}");
                    warn!("{msg}");
                    errors.push(msg);
                    None
                }
            },
            Err(e) => {
                let msg = format!("failed to fetch namespace events: {e}");
                warn!("{msg}");
                errors.push(msg);
                None
            }
        };

        let output = match events_val {
            Some(mut v) => {
                if !errors.is_empty() {
                    v["errors"] = serde_json::json!(&errors);
                }
                v
            }
            None => serde_json::json!({ "errors": &errors }),
        };

        let path = output_dir.join("events.json");
        let json = serde_json::to_string_pretty(&output).expect("Value always serializes");
        if let Err(e) = fs::write(&path, json.as_bytes()).await {
            let msg = format!("failed to write events.json: {e}");
            warn!("{msg}");
            errors.push(msg);
        }
    }

    async fn collect_resource_metrics(&self, namespace: &str, output_dir: &Path) {
        let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut errors = Vec::new();

        let pods = match pod_api.list(&Default::default()).await {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("failed to list pods for metric collection: {e}");
                warn!("{msg}");
                errors.push(msg);

                let output = serde_json::json!({ "pods": [], "errors": &errors });
                let path = output_dir.join("resource-metrics.json");
                let json = serde_json::to_string_pretty(&output).expect("Value always serializes");
                if let Err(e) = fs::write(&path, json.as_bytes()).await {
                    warn!("failed to write resource-metrics.json: {e}");
                }

                return;
            }
        };

        let nodes: BTreeSet<String> = pods
            .items
            .iter()
            .filter_map(|p| p.spec.as_ref()?.node_name.clone())
            .collect();

        let mut all_pods: Vec<PodStats> = Vec::new();

        for node in nodes.iter() {
            let req = Request::new(format!("/api/v1/nodes/{node}/proxy/stats/summary"))
                .list(&Default::default())
                .expect("stats/summary request URI is always valid");

            let text = match self.client.request_text(req).await {
                Ok(t) => t,
                Err(e) => {
                    let msg = format!("kubelet proxy unavailable for node {node}: {e}");
                    warn!("{msg}");
                    errors.push(msg);

                    continue;
                }
            };

            let summary: NodeSummary = match serde_json::from_str(&text) {
                Ok(s) => s,
                Err(e) => {
                    let msg = format!("failed to deserialize stats/summary for node {node}: {e}");
                    warn!("{msg}");
                    errors.push(msg);

                    continue;
                }
            };

            all_pods.extend(
                summary
                    .pods
                    .into_iter()
                    .filter(|p| p.pod_ref.namespace == namespace),
            );
        }

        let output = if errors.is_empty() {
            serde_json::json!({ "pods": &all_pods })
        } else {
            serde_json::json!({ "pods": &all_pods, "errors": &errors })
        };

        let path = output_dir.join("resource-metrics.json");
        let json = serde_json::to_string_pretty(&output).expect("Value always serializes");
        if let Err(e) = fs::write(&path, json.as_bytes()).await {
            let msg = format!("failed to write resource-metrics.json: {e}");
            warn!("{msg}");
            errors.push(msg);
        }
    }

    async fn get_scenario_job_window(
        &self,
        namespace: &str,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>), Error> {
        let job_api: Api<Job> = Api::namespaced(self.client.clone(), namespace);

        let job = job_api
            .get(SCENARIO_JOB_NAME)
            .await
            .map_err(|source| Error::GetScenarioJob {
                name: SCENARIO_JOB_NAME,
                source,
            })?;

        window_from_job_status(job.status.as_ref())
    }
}

/// Derives the query window from a scenario Job's status. Errors if `startTime` isn't set yet;
/// defaults `completionTime` to now (with a warning) if the job hasn't finished.
fn window_from_job_status(
    status: Option<&JobStatus>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), Error> {
    let start = status
        .and_then(|s| s.start_time.as_ref())
        .and_then(to_chrono_utc)
        .ok_or(Error::MissingJobStartTime {
            name: SCENARIO_JOB_NAME,
        })?;

    let end = status
        .and_then(|s| s.completion_time.as_ref())
        .and_then(to_chrono_utc)
        .unwrap_or_else(|| {
            warn!(
                "scenario job {SCENARIO_JOB_NAME} has no completionTime yet, \
                 defaulting end of window to now"
            );
            Utc::now()
        });

    Ok((start, end))
}

/// Converts a Kubernetes `Time` (a [`jiff::Timestamp`] wrapper) to a [`chrono::DateTime<Utc>`],
/// since `rtf_integrations::prometheus::PrometheusClient` (and the rest of this codebase) uses
/// `chrono` for timestamps.
fn to_chrono_utc(t: &Time) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(t.0.as_second(), t.0.subsec_nanosecond() as u32)
}

/// Returns `(pod_name, container_names)` for every pod in the slice.
///
/// Filtering is done upstream via a label selector on `rtf.io/log-collection=true`, so this
/// function only handles container-name extraction. Init containers are excluded.
fn pods_to_collect(pods: &[Pod]) -> Vec<(&str, Vec<String>)> {
    pods.iter()
        .filter_map(|p| {
            let name = p.metadata.name.as_deref()?;
            let containers = p
                .spec
                .iter()
                .flat_map(|s| s.containers.iter())
                .map(|c| c.name.clone())
                .collect();
            Some((name, containers))
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct NodeSummary {
    pods: Vec<PodStats>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PodStats {
    pod_ref: PodRef,
    containers: Vec<ContainerStats>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PodRef {
    name: String,
    namespace: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerStats {
    name: String,
    start_time: Option<String>,
    cpu: Option<CpuStats>,
    memory: Option<MemoryStats>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CpuStats {
    usage_nano_cores: Option<u64>,
    usage_core_nano_seconds: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryStats {
    usage_bytes: Option<u64>,
    working_set_bytes: Option<u64>,
    rss_bytes: Option<u64>,
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use std::sync::{Arc, RwLock};

    #[derive(Debug, Clone, PartialEq)]
    pub enum KubeCall {
        ApplyNamespace { name: String },
        ApplyResultsWriterServiceAccount { namespace: String },
        CheckDeploymentsAvailable { namespace: String },
        GetScenarioJobWindow { namespace: String },
    }

    struct MockState {
        calls: Vec<KubeCall>,
        should_fail: bool,
        deployments_available: bool,
    }

    impl Default for MockState {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                should_fail: false,
                deployments_available: true,
            }
        }
    }

    #[derive(Clone, Default)]
    pub struct MockClient {
        state: Arc<RwLock<MockState>>,
    }

    impl MockClient {
        pub fn failing() -> Self {
            Self {
                state: Arc::new(RwLock::new(MockState {
                    should_fail: true,
                    ..Default::default()
                })),
            }
        }

        fn should_fail(&self) -> bool {
            self.state.read().unwrap().should_fail
        }

        fn record_call(&self, call: KubeCall) {
            self.state.write().unwrap().calls.push(call);
        }

        pub fn read_calls<F>(&self, closure: F)
        where
            F: FnOnce(&[KubeCall]),
        {
            let state = self.state.read().unwrap();
            closure(&state.calls)
        }
    }

    impl Client for MockClient {
        async fn create_namespace(&self, name: &str) -> Result<(), Error> {
            if self.should_fail() {
                return Err(Error::CreateNamespace(kube::Error::Api(
                    kube::core::Status::failure("mock kube failure", "MockFailure").boxed(),
                )));
            }
            self.record_call(KubeCall::ApplyNamespace {
                name: name.to_owned(),
            });

            Ok(())
        }

        async fn create_results_writer_service_account(
            &self,
            namespace: &str,
        ) -> Result<(), Error> {
            if self.should_fail() {
                return Err(Error::CreateServiceAccount(kube::Error::Api(
                    kube::core::Status::failure("mock kube failure", "MockFailure").boxed(),
                )));
            }
            self.record_call(KubeCall::ApplyResultsWriterServiceAccount {
                namespace: namespace.to_owned(),
            });

            Ok(())
        }

        async fn check_deployment_status(
            &self,
            namespace: &str,
        ) -> Result<DeploymentStatus, Error> {
            if self.should_fail() {
                return Err(Error::ListDeployments(kube::Error::Api(
                    kube::core::Status::failure("mock kube failure", "MockFailure").boxed(),
                )));
            }
            self.record_call(KubeCall::CheckDeploymentsAvailable {
                namespace: namespace.to_owned(),
            });
            let state = self.state.read().unwrap();

            if state.deployments_available {
                Ok(DeploymentStatus {
                    total: 1,
                    not_ready: vec![],
                })
            } else {
                Ok(DeploymentStatus {
                    total: 1,
                    not_ready: vec![DeploymentInfo {
                        name: "mock-deployment".to_owned(),
                        last_condition_status: "False".to_owned(),
                    }],
                })
            }
        }

        async fn collect_container_logs(&self, _namespace: &str, _output_dir: &Path) {}

        async fn collect_namespace_events(&self, _namespace: &str, _output_dir: &Path) {}

        async fn collect_resource_metrics(&self, _namespace: &str, _output_dir: &Path) {}

        async fn get_scenario_job_window(
            &self,
            namespace: &str,
        ) -> Result<(DateTime<Utc>, DateTime<Utc>), Error> {
            if self.should_fail() {
                return Err(Error::MissingJobStartTime {
                    name: SCENARIO_JOB_NAME,
                });
            }
            self.record_call(KubeCall::GetScenarioJobWindow {
                namespace: namespace.to_owned(),
            });

            let end = Utc::now();
            Ok((end, end))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::{
        api::core::v1::{Container, PodSpec},
        apimachinery::pkg::apis::meta::v1::ObjectMeta,
        jiff,
    };
    use simple_test_case::test_case;

    fn make_pod(name: &str, containers: &[&str]) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: containers
                    .iter()
                    .map(|c| Container {
                        name: c.to_string(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn extracts_container_names_from_pods() {
        let pods = vec![make_pod("scenario", &["runner", "output-collector"])];
        let result = pods_to_collect(&pods);
        assert_eq!(
            result,
            vec![(
                "scenario",
                vec!["runner".to_owned(), "output-collector".to_owned()]
            )]
        );
    }

    #[test]
    fn pod_without_name_is_skipped() {
        let pod = Pod::default();
        assert!(pods_to_collect(&[pod]).is_empty());
    }

    #[test]
    fn init_containers_are_excluded() {
        use k8s_openapi::api::core::v1::Container;
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("mypod".to_owned()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "main".to_owned(),
                    ..Default::default()
                }],
                init_containers: Some(vec![Container {
                    name: "init".to_owned(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let pods = [pod];
        let result = pods_to_collect(&pods);
        assert_eq!(result, vec![("mypod", vec!["main".to_owned()])]);
    }

    fn job_status(start: Option<jiff::Timestamp>, end: Option<jiff::Timestamp>) -> JobStatus {
        JobStatus {
            start_time: start.map(Time),
            completion_time: end.map(Time),
            ..Default::default()
        }
    }

    #[test_case(None, Err(()); "errors when status is missing")]
    #[test_case(Some((None, Some(200))), Err(()); "errors when start time is missing")]
    #[test_case(Some((Some(100), None)), Ok((100, None)); "defaults end to now when completion time is missing")]
    #[test_case(Some((Some(100), Some(200))), Ok((100, Some(200))); "uses start and completion time when both present")]
    #[test]
    fn window_from_job_status_cases(
        status: Option<(Option<i64>, Option<i64>)>,
        expected: Result<(i64, Option<i64>), ()>,
    ) {
        let to_ts = |secs: i64| k8s_openapi::jiff::Timestamp::from_second(secs).unwrap();
        let status = status.map(|(start, end)| job_status(start.map(to_ts), end.map(to_ts)));

        let actual = window_from_job_status(status.as_ref());

        match (actual, expected) {
            (Err(Error::MissingJobStartTime { .. }), Err(())) => {}
            (Ok((actual_start, actual_end)), Ok((expected_start, expected_end))) => {
                assert_eq!(actual_start.timestamp(), expected_start);
                match expected_end {
                    Some(expected_end) => assert_eq!(actual_end.timestamp(), expected_end),
                    None => assert!(
                        (Utc::now() - actual_end).num_seconds().abs() < 5,
                        "expected end to default to ~now, got {actual_end}"
                    ),
                }
            }
            (actual, expected) => panic!("expected {expected:?}, got {actual:?}"),
        }
    }
}
