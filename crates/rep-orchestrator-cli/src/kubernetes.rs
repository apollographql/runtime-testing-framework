use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Event, Namespace, Pod, ServiceAccount},
};
use kube::{
    Api, Config,
    api::{LogParams, ObjectMeta, Patch, PatchParams, Request},
    config::{KubeConfigOptions, Kubeconfig, KubeconfigError},
};
use rep_orchestrator_shared::{EXECUTION_ID_LABEL, LOG_COLLECTION_ANNOTATION};
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

        let pods = match pod_api.list(&Default::default()).await {
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
}

fn should_collect_pod_logs(pod: &Pod) -> bool {
    // Scenario pods: always collect. The orchestrator sets rtf.io/execution-id as a pod
    // label when it creates the Job (rep-orchestrator/src/k8s/job.rs), so we check labels.
    if pod
        .metadata
        .labels
        .as_ref()
        .is_some_and(|l| l.contains_key(EXECUTION_ID_LABEL))
    {
        return true;
    }
    // Environment service pods: collect only if the user opted in. The label
    // rtf.io/log-collection flows from the docker-compose service label through kompose
    // (which writes docker-compose labels as Deployment annotations) and then the kustomize
    // patch in kustomization.yaml propagates it into the pod-template annotations — the same
    // pipeline used by rtf.io/file-providers. We therefore check annotations here.
    pod.metadata.annotations.as_ref().is_some_and(|a| {
        a.get(LOG_COLLECTION_ANNOTATION)
            .is_some_and(|v| v == "true")
    })
}

/// Returns `(pod_name, container_names)` for every pod that should have logs collected,
/// combining the collection filter with the container-selection step.
fn pods_to_collect(pods: &[Pod]) -> Vec<(&str, Vec<String>)> {
    pods.iter()
        .filter(|p| should_collect_pod_logs(p))
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::{
        api::core::v1::{Container, PodSpec},
        apimachinery::pkg::apis::meta::v1::ObjectMeta,
    };
    use std::collections::BTreeMap;

    /// Build a pod with a name, optional execution-id label, optional log-collection annotation,
    /// and a list of container names — mirroring the real pod shapes we care about.
    fn make_pod(
        name: &str,
        execution_id: Option<&str>,
        log_collection: Option<&str>,
        containers: &[&str],
    ) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                labels: execution_id
                    .map(|id| BTreeMap::from([(EXECUTION_ID_LABEL.to_owned(), id.to_owned())])),
                annotations: log_collection.map(|v| {
                    BTreeMap::from([(LOG_COLLECTION_ANNOTATION.to_owned(), v.to_owned())])
                }),
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
    fn scenario_pod_always_collects_logs() {
        let pods = vec![make_pod(
            "scenario",
            Some("exec-1"),
            None,
            &["runner", "output-collector"],
        )];
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
    fn env_pod_without_label_does_not_collect() {
        let pods = vec![make_pod("nginx", None, None, &["nginx"])];
        assert!(pods_to_collect(&pods).is_empty());
    }

    #[test]
    fn env_pod_with_label_collects_logs() {
        let pods = vec![make_pod("nginx", None, Some("true"), &["nginx"])];
        let result = pods_to_collect(&pods);
        assert_eq!(result, vec![("nginx", vec!["nginx".to_owned()])]);
    }

    #[test]
    fn mixed_namespace_collects_correct_pods() {
        let pods = vec![
            make_pod("scenario", Some("exec-1"), None, &["runner"]),
            make_pod("sidecar", None, None, &["helper"]), // no label
            make_pod("router", None, Some("true"), &["router"]), // opted in
        ];
        let names: Vec<&str> = pods_to_collect(&pods).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["scenario", "router"]);
    }

    #[test]
    fn execution_id_label_in_annotations_is_not_enough() {
        let pod = Pod {
            metadata: ObjectMeta {
                annotations: Some(BTreeMap::from([(
                    EXECUTION_ID_LABEL.to_owned(),
                    "exec-123".to_owned(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!should_collect_pod_logs(&pod));
    }

    #[test]
    fn log_collection_annotation_non_true_means_skip() {
        let pod = Pod {
            metadata: ObjectMeta {
                annotations: Some(BTreeMap::from([(
                    LOG_COLLECTION_ANNOTATION.to_owned(),
                    "false".to_owned(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!should_collect_pod_logs(&pod));
    }
}
