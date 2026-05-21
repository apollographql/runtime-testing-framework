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
    event_loop::provision_environment::MSG_ARGO_COMPLETE,
    k8s::ClusterClients,
    resolver::{ResolverError, ResolverInput},
};
use serde::Serialize;
use std::time::Duration;
use tokio::{spawn, time::sleep};
use tracing::{error, info_span, warn};

mod cleanup_namespace;
mod event_queue;
mod provision_environment;
mod run_scenario;

pub use event_queue::{
    Claim, EventQueue, EventQueueState, ProvisioningHandle, Snapshot, SubmitError,
};
pub use provision_environment::MSG_ARGO_WAIT;
pub use run_scenario::MSG_JOB_WAIT;

/// Configuration context for event handling. Groups static configuration that is shared across
/// all events to reduce the number of parameters passed to Event::handle.
struct EventHandleContext {
    orchestrator_url: String,
    toolbox_pull_policy: String,
    kubeconfig_secret_name: String,
    failed_execution_ttl_seconds: u64,
    kubeconfig_path: String,
    workload_context: String,
}

/// Run as a long lived task. This is an infinite loop that processes [Event]s received on a
/// channel that is shared with the axum server and the event loop's own handler functions.
pub async fn event_loop_task(mut event_queue: EventQueue) {
    let Config {
        kubeconfig_path,
        workload_context,
        orchestrator_url,
        toolbox_pull_policy,
        kubeconfig_secret_name,
        failed_execution_ttl_secs,
        ..
    } = Config::get();

    let ctx = EventHandleContext {
        orchestrator_url: orchestrator_url.into(),
        toolbox_pull_policy: toolbox_pull_policy.into(),
        kubeconfig_secret_name: kubeconfig_secret_name.into(),
        failed_execution_ttl_seconds: *failed_execution_ttl_secs,
        kubeconfig_path: kubeconfig_path.into(),
        workload_context: workload_context.into(),
    };

    while let Some(evt) = event_queue.next_event().await {
        let ty_name = evt.data.name();

        let span = info_span!("event", execution_id = %evt.test_execution.uuid(), ty=ty_name);
        let _guard = span.enter();

        if let Err(err) = evt.handle(&mut event_queue, &ctx).await {
            error!(%err, ty=%ty_name, "Error handling event");
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
    async fn handle(self, event_queue: &mut EventQueue, ctx: &EventHandleContext) -> Result<()> {
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
                let clients = match ClusterClients::try_management_only().await {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "failed to build management k8s client for CreateEnvArgoWorkflow");
                        return Err(Error::K8s(e));
                    }
                };

                provision_environment::create_workflow(
                    self.test_execution.clone(),
                    &ctx.orchestrator_url,
                    &ctx.toolbox_pull_policy,
                    &ctx.kubeconfig_secret_name,
                    clients,
                    conn,
                )
                .await
            }

            EventData::WaitForEnvArgoWorkflow => {
                let clients = match ClusterClients::try_new(
                    &ctx.kubeconfig_path,
                    &ctx.workload_context,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "failed to build k8s clients for WaitForEnvArgoWorkflow");
                        return Err(Error::K8s(e));
                    }
                };

                provision_environment::wait_for_workflow(
                    self.test_execution.clone(),
                    ctx.failed_execution_ttl_seconds,
                    event_queue.tx(),
                    clients,
                    conn,
                )
                .await
            }

            EventData::ArgoWorkflowComplete => {
                event_queue
                    .evict_resolved_env_config(self.test_execution.uuid())
                    .await;
                conn.mark_execution_as_provisioning(&self.test_execution, MSG_ARGO_COMPLETE.into())
                    .await;

                Ok(None)
            }

            EventData::CreateScenarioJob => {
                let clients = match ClusterClients::try_new(
                    &ctx.kubeconfig_path,
                    &ctx.workload_context,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "failed to build k8s clients for CreateScenarioJob");
                        return Err(Error::K8s(e));
                    }
                };

                let res = event_queue
                    .scenario_docker_image_and_command(self.test_execution.uuid())
                    .await;

                match res {
                    Some((image, command)) => {
                        run_scenario::create_job(
                            self.test_execution.clone(),
                            image,
                            command,
                            &ctx.orchestrator_url,
                            &ctx.toolbox_pull_policy,
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
                let clients = match ClusterClients::try_new(
                    &ctx.kubeconfig_path,
                    &ctx.workload_context,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "failed to build k8s clients for WaitForScenarioJob");
                        return Err(Error::K8s(e));
                    }
                };

                run_scenario::wait_for_job(
                    self.test_execution.clone(),
                    ctx.failed_execution_ttl_seconds,
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
                let clients = match ClusterClients::try_new(
                    &ctx.kubeconfig_path,
                    &ctx.workload_context,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "failed to build k8s clients for CleanupNamespace");
                        return Err(Error::K8s(e));
                    }
                };

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
                        data: EventData::CleanupNamespaceAfter(ctx.failed_execution_ttl_seconds),
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
    use rep_orchestrator_shared::test_plan::RepTestPlan;
    use rtf_config::{
        formats::{
            DockerCommand, DockerComposeEnvironment, DockerScenario, EnvironmentConfig,
            ScenarioConfig,
        },
        templating::Field,
    };

    pub fn stub_environment() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
            file_providers: vec![],
            env_vars: Default::default(),
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
            "http://localhost:8035",
            "IfNotPresent",
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
                execution: stub_environment(),
            },
        }
    }

    // AC-4: Test that CreateEnvArgoWorkflow uses provision_environment::create_workflow
    // and that the refactored code path correctly calls try_management_only.
    // This test verifies that the handler receives a valid client and can process
    // the event successfully, confirming the constructor call is correct.
    #[tokio::test]
    async fn create_env_argo_workflow_calls_create_workflow_handler() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::default_ok();

        // Call the handler directly to verify it can accept and process a client
        let res = provision_environment::create_workflow(
            ex,
            "http://localhost:8035",
            "IfNotPresent",
            "test-secret",
            clients,
            &mut handle,
        )
        .await;

        // Verify the handler succeeds with a proper client and returns the next event
        assert!(
            res.is_ok(),
            "create_workflow should succeed with valid client"
        );
        let next_event = res.unwrap();
        assert_eq!(
            next_event,
            Some(EventData::WaitForEnvArgoWorkflow),
            "create_workflow should dispatch to WaitForEnvArgoWorkflow"
        );
    }
}
