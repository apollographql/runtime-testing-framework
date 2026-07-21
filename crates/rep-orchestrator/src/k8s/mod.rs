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
    #[error("Maximum retry window exceeded")]
    MaxRetriesExceeded,

    #[error("Kube error: {0}")]
    Kube(#[from] kube::Error),

    #[error("Kubeconfig error: {0}")]
    KubeConfig(#[from] KubeconfigError),

    #[error("In-cluster config error: {0}")]
    InCluster(#[from] InClusterError),
}

impl Error {
    pub fn is_409_conflict(&self) -> bool {
        match self {
            Self::Kube(kube::Error::Api(status)) => status.code == 409,
            _ => false,
        }
    }

    /// Transient (network/auth/server-side) errors worth retrying after rebuilding the workload
    /// client. Excludes 403: GKE returns 401, not 403, for an expired token, so 403 more likely
    /// indicates a genuine RBAC misconfiguration that a retry will never resolve.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Kube(kube::Error::Api(status)) => matches!(status.code, 401 | 429 | 500..=599),
            Self::Kube(kube::Error::HyperError(_) | kube::Error::Service(_)) => true,
            _ => false,
        }
    }
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
        &mut self,
        ns: &str,
        name: &str,
        execution_id: &Uuid,
        spec: JobSpec,
    ) -> impl Future<Output = Result<Job>> + Send;

    /// Wait for a k8s [Job] running within the workload cluster to reach a terminal state,
    /// selecting the job by its execution ID label.
    fn wait_for_job(
        &mut self,
        ns: &str,
        execution_id: &Uuid,
    ) -> impl Future<Output = WatchOutcome> + Send;

    /// Delete an ephemeral namespace within the workload cluster.
    fn delete_workload_namespace(&mut self, ns: &str) -> impl Future<Output = Result<()>> + Send;
}

/// Kubernetes API actions that interact with both the management and workload clusters.
pub trait FullClient: ManagementClient + WorkloadClient {
    /// Wait for a [Workflow] running within the management cluster to reach a terminal state,
    /// selecting the workflow by its execution ID label. The workload cluster is also watched
    /// so that pods stuck in an unrunnable state can short-circuit the wait.
    fn wait_for_workflow(
        &mut self,
        execution_id: &Uuid,
    ) -> impl Future<Output = WatchOutcome> + Send;
}

/// Terminal states for argo [Workflow]s and k8s [Job]s.
#[derive(Debug, Clone)]
pub enum WatchOutcome {
    Succeeded,
    Failed(String),
    ContainerUnrunnable(String),
    WatchErrors(String),
}

impl fmt::Display for WatchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Succeeded => write!(f, "Succeeded"),
            Self::Failed(msg) => write!(f, "Failed ({msg})"),
            Self::ContainerUnrunnable(reason) => {
                write!(f, "pod stuck in unrunnable waiting state: {reason}")
            }
            Self::WatchErrors(msg) => {
                write!(f, "errors watching for status ({msg})")
            }
        }
    }
}

pub fn workflow_name(execution_id: &Uuid) -> String {
    format!("provision-env-{execution_id}")
}

pub fn env_configmap_name(execution_id: &Uuid) -> String {
    format!("environment-config-{execution_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::error::Status;
    use simple_test_case::test_case;

    fn api_error(code: u16) -> Error {
        Error::Kube(kube::Error::Api(
            Status {
                code,
                ..Default::default()
            }
            .boxed(),
        ))
    }

    #[test_case(401, true; "401 unauthorized is retryable")]
    #[test_case(429, true; "429 too many requests is retryable")]
    #[test_case(500, true; "500 internal server error is retryable")]
    #[test_case(599, true; "599 top of the server error range is retryable")]
    #[test_case(403, false; "403 forbidden is not retryable")]
    #[test_case(404, false; "404 not found is not retryable")]
    #[test_case(200, false; "success codes are not retryable")]
    #[test]
    fn is_retryable_classifies_api_error_codes(code: u16, expected: bool) {
        assert_eq!(api_error(code).is_retryable(), expected);
    }

    #[test]
    fn is_retryable_treats_transport_errors_as_retryable() {
        let err = Error::Kube(kube::Error::Service(Box::new(std::io::Error::other(
            "connection reset",
        ))));
        assert!(err.is_retryable());
    }
}
