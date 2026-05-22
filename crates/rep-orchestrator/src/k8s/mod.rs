use k8s_openapi::api::batch::v1::{Job, JobSpec};
use kube::config::{InClusterError, KubeconfigError};
use std::fmt;
use uuid::Uuid;

mod client;
mod job;
mod workflow;

#[cfg(test)]
pub mod mock_client;

pub use client::ClusterClients;
pub use job::scenario_job;
pub use workflow::{
    Dag, MainTemplate, TaskSpec, TaskTemplate, TemplateDef, Workflow, WorkflowSpec,
};

/// Binary name of the REP orchestrator CLI, available on `PATH` inside [TOOLBOX_IMAGE].
const CLI_BINARY: &str = "rep-orchestrator-cli";
pub(crate) const OUTPUT_COLLECTOR: &str = "output-collector";
pub const CLUSTER_API_NAMESPACE: &str = "cluster-api";
pub const TOOLBOX_IMAGE: &str =
    "us-central1-docker.pkg.dev/platform-cross-environment/apollo-private-docker/rtf-toolbox:edge";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Kube error: {0}")]
    Kube(#[from] kube::Error),

    #[error("Kubeconfig error: {0}")]
    KubeConfig(#[from] KubeconfigError),

    #[error("In-cluster config error: {0}")]
    InCluster(#[from] InClusterError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Kubernetes API actions that only interact with the management cluster.
pub trait ManagementClient: Clone + Send + Sync + 'static {
    /// Create a new argo [Workflow] in the management cluster for provisioning an ephemeral
    /// namespace in the workload cluster and deploying services into it as defined by an RTF
    /// Test Plan.
    fn create_argo_workflow(
        &self,
        execution_id: &Uuid,
        spec: WorkflowSpec,
    ) -> impl Future<Output = Result<Workflow>> + Send;
}

/// Kubernetes API actions that only interact with the workload cluster.
pub trait WorkloadClient: Clone + Send + Sync + 'static {
    /// Create a new k8s [Job] for running an RTF Scenario in an ephemeral namespace within the
    /// workload cluster.
    fn create_job(
        &self,
        ns: &str,
        name: &str,
        execution_id: &Uuid,
        spec: JobSpec,
    ) -> impl Future<Output = Result<Job>> + Send;

    /// Wait for a k8s [Job] running within the workload cluster to reach a terminal state,
    /// selecting the job by its execution ID label.
    fn wait_for_job(
        &self,
        ns: &str,
        execution_id: &Uuid,
    ) -> impl Future<Output = WatchOutcome> + Send;

    /// Delete an ephemeral namespace within the workload cluster.
    fn delete_workload_namespace(&self, ns: &str) -> impl Future<Output = Result<()>> + Send;
}

/// Kubernetes API actions that interact with both the management and workload clusters.
pub trait FullClient: ManagementClient + WorkloadClient {
    /// Wait for a [Workflow] running within the management cluster to reach a terminal state,
    /// selecting the workflow by its execution ID label. The workload cluster is also watched
    /// so that pods stuck in an unrunnable state can short-circuit the wait.
    fn wait_for_workflow(&self, execution_id: &Uuid) -> impl Future<Output = WatchOutcome> + Send;
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
