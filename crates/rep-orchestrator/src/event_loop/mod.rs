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
    config::Config,
    conn,
    db::{TestExecution, UpdateHandle},
    event_loop::{provision_environment::MSG_ARGO_COMPLETE, run_scenario::CreateJobConfig},
    k8s::ClusterClients,
    resolver::{ResolverError, ResolverInput},
};
use rep_orchestrator_shared::OtelConfig;
use serde::Serialize;
use std::time::Duration;
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
pub use run_scenario::MSG_JOB_WAIT;

/// Static configuration shared across all event handler arms in the event loop.
struct EventLoopConfig<'a> {
    orchestrator_url: &'a str,
    prometheus_endpoint: &'a str,
    toolbox_pull_policy: &'a str,
    otel: &'a OtelConfig,
    kubeconfig_secret_name: &'a str,
    failed_execution_ttl_seconds: u64,
    kubeconfig_path: &'a str,
    workload_context: &'a str,
}

/// Run as a long lived task. This is an infinite loop that processes [Event]s received on a
/// channel that is shared with the axum server and the event loop's own handler functions.
pub async fn event_loop_task(mut event_queue: EventQueue) {
    let Config {
        kubeconfig_path,
        workload_context,
        orchestrator_url,
        prometheus_endpoint,
        toolbox_pull_policy,
        otel_collector_grpc,
        otel_collector_http,
        kubeconfig_secret_name,
        failed_execution_ttl_secs,
        ..
    } = Config::get();

    let cfg = EventLoopConfig {
        orchestrator_url,
        prometheus_endpoint,
        toolbox_pull_policy,
        otel: &OtelConfig {
            grpc: otel_collector_grpc.to_string(),
            http: otel_collector_http.to_string(),
        },
        kubeconfig_secret_name,
        failed_execution_ttl_seconds: *failed_execution_ttl_secs,
        kubeconfig_path,
        workload_context,
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
    MarkUnrunnable(String),
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
/// execution as we process it.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub test_execution: TestExecution,
    pub data: EventData,
}

impl Event {
    async fn handle(self, event_queue: &mut EventQueue, cfg: &EventLoopConfig<'_>) -> Result<()> {
        let conn = conn!();
        let cleanup_on_error = self.data.requires_cleanup_on_error();

        let res = match self.data {
            EventData::ResolveConfig => {
                if let Err(e) =
                    event_queue.send_to_resolver(ResolverInput::ResolveConfig(self.test_execution))
                {
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

                provision_environment::create_workflow(
                    self.test_execution.clone(),
                    &environment,
                    cfg,
                    clients,
                    conn,
                )
                .await
            }

            EventData::WaitForEnvArgoWorkflow => {
                let clients = ClusterClients::try_new_full(
                    cfg.kubeconfig_path,
                    cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build k8s clients for WaitForEnvArgoWorkflow"),
                )?;

                provision_environment::wait_for_workflow(
                    self.test_execution.clone(),
                    cfg.failed_execution_ttl_seconds,
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
                let clients = ClusterClients::try_new_workload(
                    cfg.kubeconfig_path,
                    cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build workload k8s client for CreateScenarioJob"),
                )?;

                let res = event_queue
                    .scenario_docker_image_and_command(self.test_execution.uuid())
                    .await;

                match res {
                    Some((image, command)) => {
                        run_scenario::create_job(
                            self.test_execution.clone(),
                            image,
                            command,
                            &CreateJobConfig {
                                orchestrator_url: cfg.orchestrator_url,
                                prometheus_endpoint: cfg.prometheus_endpoint,
                                toolbox_pull_policy: cfg.toolbox_pull_policy,
                            },
                            clients,
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
                let clients = ClusterClients::try_new_workload(
                    cfg.kubeconfig_path,
                    cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build workload k8s client for WaitForScenarioJob"),
                )?;

                run_scenario::wait_for_job(
                    self.test_execution.clone(),
                    cfg.failed_execution_ttl_seconds,
                    event_queue.tx(),
                    clients,
                    conn,
                )
                .await
            }

            EventData::CleanupNamespaceAfter(ttl_secs) => {
                let tx = event_queue.tx();
                let test_execution = self.test_execution.clone();

                spawn(async move {
                    sleep(Duration::from_secs(ttl_secs)).await;
                    _ = tx.send(Event {
                        test_execution,
                        data: EventData::CleanupNamespace,
                    });
                });

                Ok(None)
            }

            EventData::CleanupNamespace => {
                let clients = ClusterClients::try_new_workload(
                    cfg.kubeconfig_path,
                    cfg.workload_context,
                )
                .await
                .inspect_err(
                    |e| error!(%e, "failed to build workload k8s client for CleanupNamespace"),
                )?;

                let res = cleanup_namespace::try_run(self.test_execution.clone(), clients).await;
                if let Some(run_uuid) = event_queue
                    .mark_execution_complete(self.test_execution.uuid())
                    .await
                {
                    conn.clear_cached_payload_for_run(run_uuid).await;
                }

                res
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
                    data: next_event_data,
                });
            }

            Ok(None) => (),

            Err(e) => {
                conn.mark_execution_as_unrunnable(&self.test_execution, e.to_string())
                    .await;

                if cleanup_on_error {
                    let _ = event_queue.tx().send(Event {
                        test_execution: self.test_execution,
                        data: EventData::CleanupNamespaceAfter(cfg.failed_execution_ttl_seconds),
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
    use rep_orchestrator_shared::test_plan::{RepEnvironment, RepTestPlan};
    use rtf_config::{
        formats::{
            DockerCommand, DockerComposeEnvironment, DockerScenario, EnvironmentConfig,
            OutputCollection, ScenarioConfig,
        },
        templating::Field,
    };

    pub fn stub_environment() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
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
        let clients = MockClient::default_ok();

        let res = run_scenario::create_job(
            ex,
            "nginx".to_string(),
            "echo test".to_string(),
            &CreateJobConfig {
                orchestrator_url: "http://localhost:8035",
                prometheus_endpoint: "http://prometheus:9090",
                toolbox_pull_policy: "IfNotPresent",
            },
            clients,
            &mut handle,
        )
        .await;

        assert!(
            res.is_ok(),
            "create_job with embedded image/command: {res:?}"
        );
    }

    pub fn stub_test_plan() -> RepTestPlan {
        RepTestPlan {
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
                execution: RepEnvironment::DockerCompose(stub_environment()),
            },
        }
    }
}
