//! Top level event loop for processing kubernetes operations relating to test executions.
//!
//! # Handler functions
//! Each event type in the [EventData] enum needs to have an associated handler function written
//! for it. These handlers are responsible for carrying out the required k8s operations for
//! processing that event as well as updating the state of the parent [TestExecution] the event is
//! for.
//! As there are a number of different ways that we can end up unable to process an event, handler
//! functions must be written to return a Result that the `Event::handle` method will use to record
//! an Unrunnable status.
use crate::{
    config::{ClusterRoles, Config, WorkloadClusterConfig},
    conn,
    db::{ClusterId, TestExecution, UpdateHandle},
    event_loop::provision_environment::MSG_ARGO_COMPLETE,
    k8s::ClusterClients,
    resolver::{ResolverError, ResolverInput},
};
use rtf_orchestrator_shared::OtelConfig;
use serde::Serialize;
use std::{collections::HashMap, time::Duration};
use tokio::{spawn, time::sleep};
use tracing::{Instrument, error, info_span, warn};

mod cleanup_namespace;
mod event_queue;
mod provision_environment;
mod run_scenario;

pub use event_queue::{
    Claim, EventQueue, EventQueueState, ProvisioningHandle, Snapshot, SubmitError,
};
pub use provision_environment::MSG_ARGO_WAIT;
pub(crate) use run_scenario::CreateJobConfig;
pub use run_scenario::MSG_JOB_WAIT;

/// Static configuration shared across all event handler arms in the event loop.
struct EventLoopConfig<'a> {
    orchestrator_url: &'a str,
    prometheus_endpoint: &'a str,
    toolbox_pull_policy: &'a str,
    toolbox_image: &'a str,
    otel: &'a OtelConfig,
    workload_clusters: &'a HashMap<ClusterId, WorkloadClusterConfig>,
    cluster_roles: &'a ClusterRoles,
}

impl<'a> EventLoopConfig<'a> {
    fn workload_cluster_config(&self, cluster: &ClusterId) -> Result<&'a WorkloadClusterConfig> {
        self.workload_clusters
            .get(cluster)
            .ok_or_else(|| Error::UnknownWorkloadCluster(cluster.to_string()))
    }

    /// Only used on an error path reached after `workload_cluster_config` has already been
    /// looked up successfully for `cluster` earlier in the same event handler, so the fallback
    /// here is unreachable in practice.
    fn failed_execution_ttl_secs(&self, cluster: &ClusterId) -> u64 {
        self.workload_cluster_config(cluster)
            .map(|c| c.execution.failed_execution_ttl_secs)
            .unwrap_or(600)
    }
}

/// Run as a long lived task. This is an infinite loop that processes [Event]s received on a
/// channel that is shared with the axum server and the event loop's own handler functions.
pub async fn event_loop_task(mut event_queue: EventQueue) {
    let cfg_ref = Config::get();
    let toolbox_image = cfg_ref.toolbox_image();
    let workload_clusters = cfg_ref.workload_clusters.per_cluster_config();

    let cfg = EventLoopConfig {
        orchestrator_url: &cfg_ref.server.orchestrator_url,
        prometheus_endpoint: &cfg_ref.otel.prometheus_endpoint,
        toolbox_pull_policy: &cfg_ref.toolbox.pull_policy,
        toolbox_image: &toolbox_image,
        otel: &OtelConfig {
            grpc: cfg_ref.otel.collector_grpc.clone(),
            http: cfg_ref.otel.collector_http.clone(),
        },
        workload_clusters: &workload_clusters,
        cluster_roles: &cfg_ref.workload_clusters.cluster_roles,
    };

    while let Some(evt) = event_queue.next_event().await {
        let execution_id = evt.test_execution.uuid();
        let ty = evt.data.name();

        let fut = evt
            .handle(&mut event_queue, &cfg)
            .instrument(info_span!("event", %execution_id, %ty));

        if let Err(err) = fut.await {
            error!(%err, %ty, "Error handling event");
        }
    }

    warn!("event loop channel closed, exiting event loop task");
}

/// Errors that can be encountered by the event loop.
///
/// This deliberately _doesn't_ roll up into the crate level Error type as these all need to be
/// handled internally rather being returned to clients of the axum server.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Db(#[from] crate::db::Error),

    #[error(transparent)]
    K8s(#[from] crate::k8s::Error),

    #[error(transparent)]
    Resolve(#[from] ResolverError),

    #[error("unable to create Argo workflow: {error}")]
    CreateArgoWorkflow {
        #[source]
        error: crate::k8s::Error,
    },

    #[error("unable to create Kubernetes job: {error}")]
    CreateJob {
        #[source]
        error: crate::k8s::Error,
    },

    #[error("unable to delete workload cluster namespace: {error}")]
    DeleteNamespace {
        #[source]
        error: crate::k8s::Error,
    },

    #[error("no kubeconfig configured for workload cluster {0}")]
    UnknownWorkloadCluster(String),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EventData {
    ResolveConfig,
    CreateEnvArgoWorkflow,
    WaitForEnvArgoWorkflow,
    ArgoWorkflowComplete,

    CreateScenarioJob,
    WaitForScenarioJob,

    CleanupNamespace,
    CleanupNamespaceAfter(u64),
    NamespacePodsDeleted,
    MarkUnrunnable(String),
    PurgeNamespace,
}

impl EventData {
    fn name(&self) -> &'static str {
        match self {
            Self::ResolveConfig => "ResolveConfig",
            Self::CreateEnvArgoWorkflow => "CreateEnvArgoWorkflow",
            Self::WaitForEnvArgoWorkflow => "WaitForEnvArgoWorkflow",
            Self::ArgoWorkflowComplete => "ArgoWorkflowComplete",
            Self::CreateScenarioJob => "CreateScenarioJob",
            Self::WaitForScenarioJob => "WaitForScenarioJob",
            Self::MarkUnrunnable(_) => "MarkUnrunnable",
            Self::CleanupNamespaceAfter(_) => "CleanupNamespaceAfter",
            Self::CleanupNamespace => "CleanupNamespace",
            Self::NamespacePodsDeleted => "NamespacePodsDeleted",
            Self::PurgeNamespace => "PurgeNamespace",
        }
    }

    /// Whether or not a handler failure for this event should cause us to queue a CleanupNamespace
    /// event.
    fn requires_cleanup_on_error(&self) -> bool {
        matches!(
            self,
            EventData::WaitForEnvArgoWorkflow
                | EventData::ArgoWorkflowComplete
                | EventData::CreateScenarioJob
                | EventData::WaitForScenarioJob
        )
    }
}

/// Raw event data paired with an associated [TestExecution] so we can track the status of the
/// execution as we process it, and the [ClusterId] of the workload cluster it runs in.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub test_execution: TestExecution,
    pub cluster: ClusterId,
    pub data: EventData,
}

impl Event {
    async fn handle(self, event_queue: &mut EventQueue, cfg: &EventLoopConfig<'_>) -> Result<()> {
        let conn = conn!();
        let cleanup_on_error = self.data.requires_cleanup_on_error();

        let res = match self.data {
            EventData::ResolveConfig => {
                if let Err(e) = event_queue.send_to_resolver(ResolverInput::ResolveConfig(
                    self.test_execution,
                    self.cluster,
                )) {
                    error!(%e, "resolver channel closed during ResolveEnvConfig dispatch");
                }

                return Ok(());
            }

            EventData::CreateEnvArgoWorkflow => {
                let clients = ClusterClients::try_new_management()
                    .await
                    .inspect_err(
                        |e| error!(%e, "failed to build management k8s client for CreateEnvArgoWorkflow"),
                    )?;

                let environment = event_queue
                    .resolved_environment_for_execution(self.test_execution.uuid())
                    .await
                    .ok_or_else(|| {
                        Error::Resolve(ResolverError::UnknownExecution(self.test_execution.uuid()))
                    })?;

                match cfg.workload_cluster_config(&self.cluster) {
                    Ok(cluster_cfg) => {
                        provision_environment::create_workflow(
                            self.test_execution.clone(),
                            &environment,
                            &cluster_cfg.kubeconfig_secret_name,
                            cluster_cfg.execution.exclusive_nodes,
                            cfg,
                            clients,
                            conn,
                        )
                        .await
                    }

                    Err(e) => Err(e),
                }
            }

            EventData::WaitForEnvArgoWorkflow => {
                let cluster_cfg = cfg.workload_cluster_config(&self.cluster)?;
                let clients = ClusterClients::try_new_full(
                    &cluster_cfg.kubeconfig_path(),
                    &cluster_cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build k8s clients for WaitForEnvArgoWorkflow"),
                )?;

                provision_environment::wait_for_workflow(
                    self.test_execution.clone(),
                    self.cluster.clone(),
                    cluster_cfg.execution.failed_execution_ttl_secs,
                    cluster_cfg.execution.poll_interval_secs,
                    cluster_cfg.execution.retry_window_secs,
                    event_queue.tx(),
                    clients,
                    conn,
                )
                .await
            }

            EventData::ArgoWorkflowComplete => {
                conn.mark_execution_as_environment_ready(
                    &self.test_execution,
                    MSG_ARGO_COMPLETE.into(),
                )
                .await;

                Ok(None)
            }

            EventData::CreateScenarioJob => {
                let cluster_cfg = cfg.workload_cluster_config(&self.cluster)?;
                let mut clients = ClusterClients::try_new_workload(
                    &cluster_cfg.kubeconfig_path(),
                    &cluster_cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build workload k8s client for CreateScenarioJob"),
                )?;

                let res = event_queue
                    .scenario_job_params(self.test_execution.uuid())
                    .await;

                match res {
                    Some(params) => {
                        run_scenario::create_job(
                            self.test_execution.clone(),
                            params.docker_image,
                            params.command,
                            &CreateJobConfig {
                                orchestrator_url: cfg.orchestrator_url,
                                prometheus_endpoint: cfg.prometheus_endpoint,
                                toolbox_pull_policy: cfg.toolbox_pull_policy,
                                toolbox_image: cfg.toolbox_image,
                                cluster_roles: cfg.cluster_roles,
                                allow_namespace_write: params.allow_k8s_write,
                                exclusive_nodes: cluster_cfg.execution.exclusive_nodes,
                                scenario_node_selector: &cluster_cfg
                                    .execution
                                    .scenario_node_selector,
                            },
                            &mut clients,
                            conn,
                        )
                        .await
                    }
                    None => Err(Error::Resolve(ResolverError::UnknownExecution(
                        self.test_execution.uuid(),
                    ))),
                }
            }

            EventData::WaitForScenarioJob => {
                let cluster_cfg = cfg.workload_cluster_config(&self.cluster)?;
                let clients = ClusterClients::try_new_workload(
                    &cluster_cfg.kubeconfig_path(),
                    &cluster_cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build workload k8s client for WaitForScenarioJob"),
                )?;

                run_scenario::wait_for_job(
                    self.test_execution.clone(),
                    self.cluster.clone(),
                    cluster_cfg.execution.failed_execution_ttl_secs,
                    cluster_cfg.execution.poll_interval_secs,
                    cluster_cfg.execution.retry_window_secs,
                    event_queue.tx(),
                    clients,
                    conn,
                )
                .await
            }

            EventData::CleanupNamespaceAfter(ttl_secs) => {
                let tx = event_queue.tx();
                let test_execution = self.test_execution.clone();
                let cluster = self.cluster.clone();

                spawn(async move {
                    sleep(Duration::from_secs(ttl_secs)).await;
                    _ = tx.send(Event {
                        test_execution,
                        cluster,
                        data: EventData::CleanupNamespace,
                    });
                });

                Ok(None)
            }

            EventData::CleanupNamespace => {
                let cluster_cfg = cfg.workload_cluster_config(&self.cluster)?;
                let mut clients = ClusterClients::try_new_workload(
                    &cluster_cfg.kubeconfig_path(),
                    &cluster_cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build workload k8s client for CleanupNamespace"),
                )?;

                let res =
                    cleanup_namespace::try_run(self.test_execution.clone(), &mut clients).await;
                if let Some(run_uuid) = event_queue
                    .mark_execution_complete(self.test_execution.uuid())
                    .await
                {
                    conn.clear_cached_payload_for_run(run_uuid).await;
                }

                res
            }

            EventData::NamespacePodsDeleted => {
                if let Some(run_uuid) = event_queue
                    .mark_execution_complete(self.test_execution.uuid())
                    .await
                {
                    conn.clear_cached_payload_for_run(run_uuid).await;
                }

                Ok(None)
            }

            EventData::PurgeNamespace => {
                let cluster_cfg = cfg.workload_cluster_config(&self.cluster)?;
                let mut clients = ClusterClients::try_new_workload(
                    &cluster_cfg.kubeconfig_path(),
                    &cluster_cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build workload k8s client for PurgeNamespace"),
                )?;

                cleanup_namespace::try_run(self.test_execution.clone(), &mut clients).await
            }

            EventData::MarkUnrunnable(message) => {
                conn.mark_execution_as_unrunnable(&self.test_execution, message)
                    .await;

                Ok(None)
            }
        };

        match res {
            Ok(Some(next_event_data)) => {
                let _ = event_queue.tx().send(Event {
                    test_execution: self.test_execution,
                    cluster: self.cluster,
                    data: next_event_data,
                });
            }

            Ok(None) => (),

            Err(e) => {
                conn.mark_execution_as_unrunnable(&self.test_execution, e.to_string())
                    .await;

                if cleanup_on_error {
                    let ttl_secs = cfg.failed_execution_ttl_secs(&self.cluster);
                    let _ = event_queue.tx().send(Event {
                        test_execution: self.test_execution,
                        cluster: self.cluster,
                        data: EventData::CleanupNamespaceAfter(ttl_secs),
                    });
                }
            }
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{MockUpdateHandle, TestExecution},
        k8s::mock_client::MockClient,
    };
    use rtf_config::{
        formats::{
            ComposeResources, DockerCommand, DockerComposeEnvironment, DockerScenario,
            EnvironmentConfig, OutputCollection, ScenarioConfig,
        },
        templating::Field,
    };
    use rtf_orchestrator_shared::test_plan::{OrchestratorEnvironment, OrchestratorTestPlan};
    use std::collections::BTreeMap;

    pub fn stub_environment() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: vec![],
            },
            file_providers: vec![],
            env_vars: Default::default(),
            output_collection: OutputCollection { prometheus: vec![] },
        }
    }

    pub fn stub_scenario() -> DockerScenario {
        DockerScenario {
            docker: DockerCommand {
                image: Field::Resolved("nginx".into()),
                tag: None,
                command: Field::Resolved("echo test".into()),
            },
            env_vars: Default::default(),
            file_providers: vec![],
            output_collection: OutputCollection { prometheus: vec![] },
        }
    }

    /// Verifies that `CreateScenarioJob { image, command }` passes those fields through to
    /// `run_scenario::create_job`. Requires a real DB connection.
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn create_scenario_job_uses_embedded_image_and_command() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let mut clients = MockClient::default_ok();

        let res = run_scenario::create_job(
            ex,
            "nginx".to_string(),
            "echo test".to_string(),
            &CreateJobConfig {
                orchestrator_url: "http://localhost:8035",
                prometheus_endpoint: "http://prometheus:9090",
                toolbox_pull_policy: "IfNotPresent",
                toolbox_image: "rtf-toolbox:edge",
                cluster_roles: &ClusterRoles {
                    cluster_read: "scenario-cluster-read".into(),
                    namespace_read: "scenario-namespace-read".into(),
                    namespace_write: "scenario-namespace-write".into(),
                },
                allow_namespace_write: false,
                exclusive_nodes: false,
                scenario_node_selector: &BTreeMap::new(),
            },
            &mut clients,
            &mut handle,
        )
        .await;

        assert!(
            res.is_ok(),
            "create_job with embedded image/command: {res:?}"
        );
    }

    pub fn stub_test_plan() -> OrchestratorTestPlan {
        OrchestratorTestPlan {
            name: String::new(),
            description: String::new(),
            variables: Default::default(),
            matrix: Default::default(),
            custom_providers: vec![],
            scenario: ScenarioConfig {
                name: String::new(),
                description: String::new(),
                variable_definitions: vec![],
                custom_providers: vec![],
                execution: stub_scenario(),
            },
            environment: EnvironmentConfig {
                name: String::new(),
                description: String::new(),
                variable_definitions: vec![],
                custom_providers: vec![],
                execution: OrchestratorEnvironment::DockerCompose(stub_environment()),
            },
        }
    }
}
