use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::ConfigMap,
};
use kube::config::KubeconfigError;
use std::fmt;
use uuid::Uuid;

mod client;
mod job;
mod workflow;

#[cfg(test)]
pub mod mock_client;

pub use client::ClusterClients;
pub use job::{CONFIG_MAP_NAME_SCENARIO, scenario_job};
pub use workflow::{
    Dag, MainTemplate, TaskSpec, TaskTemplate, TemplateDef, Workflow, WorkflowSpec,
};

pub const CLUSTER_API_NAMESPACE: &str = "cluster-api";
pub const ENVIRONMENT_CONFIG_FILENAME: &str = "environment.yaml";
pub const EXECUTION_ID_LABEL: &str = "rtf.io/execution-id";
pub const SCENARIO_CONFIG_FILENAME: &str = "scenario.yaml";
pub const TOOLBOX_IMAGE: &str =
    "us-central1-docker.pkg.dev/platform-cross-environment/platform-docker/rtf-toolbox:edge";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Kube error: {0}")]
    Kube(#[from] kube::Error),

    #[error("Kubeconfig error: {0}")]
    KubeConfig(#[from] KubeconfigError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Kubernetes API actions required for executing RTF test plans inside of REP clusters.
pub trait Client: Clone + Send + Sync + 'static {
    /// Create a new config map in either the [management][Cluster::Management] or
    /// [workload][Cluster::Workload] cluster.
    fn create_configmap(
        &self,
        cluster: Cluster,
        namespace: &str,
        configmap_name: &str,
        file_name: &str,
        content: String,
    ) -> impl Future<Output = Result<ConfigMap>> + Send;

    /// Create a new argo [Workflow] in the [management][Cluster::Management] cluster for
    /// provisioning an ephemeral namespace in the [workload][Cluster::Workload] cluster and
    /// deploying services into it as defined by an RTF Test Plan.
    fn create_argo_workflow(
        &self,
        execution_id: &Uuid,
        spec: WorkflowSpec,
    ) -> impl Future<Output = Result<Workflow>> + Send;

    /// Create a new k8s [Job] for running an RTF Scenario in an ephemeral namespace within the
    /// [workload][Cluster::Workload] cluster.
    fn create_job(
        &self,
        ns: &str,
        name: &str,
        execution_id: &Uuid,
        spec: JobSpec,
    ) -> impl Future<Output = Result<Job>> + Send;

    /// Wait for a [Workflow] running within the [management][Cluster::Management] cluster to reach
    /// a terminal state, selecting the workflow by its execution ID label.
    fn wait_for_workflow(&self, execution_id: &Uuid) -> impl Future<Output = WatchOutcome> + Send;

    /// Wait for a k8s [Job] running within the [workload][Cluster::Workload] cluster to reach
    /// a terminal state, selecting the job by its execution ID label.
    fn wait_for_job(
        &self,
        ns: &str,
        execution_id: &Uuid,
    ) -> impl Future<Output = WatchOutcome> + Send;

    /// Delete an ephemeral namespace within the [workload][Cluster::Workload].
    fn delete_workload_namespace(&self, ns: &str) -> impl Future<Output = Result<()>> + Send;
}

/// Markers for the two REP clusters we use for running test plans.
#[derive(Debug, Clone, Copy)]
pub enum Cluster {
    Management,
    Workload,
}

/// Terminal states for argo [Workflow]s and k8s [Job]s.
#[derive(Debug, Clone)]
pub enum WatchOutcome {
    Succeeded,
    Failed(String),
    ContainerUnrunnable(String),
    WatcherError(String),
    StreamClosed,
}

impl fmt::Display for WatchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Succeeded => write!(f, "Succeeded"),
            Self::Failed(msg) => write!(f, "Failed ({msg})"),
            Self::ContainerUnrunnable(reason) => {
                write!(f, "pod stuck in unrunnable waiting state: {reason}")
            }
            Self::WatcherError(msg) => write!(f, "Watcher error ({msg})"),
            Self::StreamClosed => write!(f, "Watcher stream closed unexpectedly"),
        }
    }
}

pub fn workflow_name(execution_id: &Uuid) -> String {
    format!("provision-env-{execution_id}")
}

pub fn env_configmap_name(execution_id: &Uuid) -> String {
    format!("environment-config-{execution_id}")
}
