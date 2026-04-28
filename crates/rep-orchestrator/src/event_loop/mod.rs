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
};
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use tracing::{error, warn};

mod cleanup_namespace;
mod event_queue;
mod provision_environment;
mod run_scenario;

pub use event_queue::{Claim, EventQueue, EventQueueState, ProvisioningHandle, SubmitError};
pub use provision_environment::MSG_ARGO_WAIT;
pub use run_scenario::MSG_JOB_WAIT;

/// Run as a long lived task. This is an infinite loop that processes [Event]s received on a
/// channel that is shared with the axum server and the event loop's own handler functions.
pub async fn event_loop_task(mut event_queue: EventQueue) {
    let Config {
        kubeconfig_path,
        mgmt_context,
        workload_context,
        orchestrator_url,
        kubeconfig_secret_name,
        ..
    } = Config::get();

    let clients = match mgmt_context {
        Some(ctx) => ClusterClients::try_new(kubeconfig_path, ctx, workload_context).await,
        None => {
            ClusterClients::try_new_in_cluster_management(kubeconfig_path, workload_context).await
        }
    }
    .unwrap_or_else(|e| panic!("failed to initialise k8s clients, event loop cannot start: {e}"));

    while let Some(evt) = event_queue.next_event().await {
        let ty_name = evt.data.name();

        if let Err(err) = evt
            .handle(
                &event_queue,
                orchestrator_url,
                kubeconfig_secret_name,
                clients.clone(),
            )
            .await
        {
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

    #[error("unable to create Argo workflow: {error}")]
    CreateArgoWorkflow {
        #[source]
        error: crate::k8s::Error,
    },

    #[error("unable to create {kind} configmap: {error}")]
    CreateConfigmap {
        kind: &'static str,
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

#[derive(Debug, Clone, PartialEq)]
pub enum EventData {
    CreateEnvConfigMap(DockerComposeEnvironment, DockerScenario),
    CreateEnvArgoWorkflow(DockerScenario),
    WaitForEnvArgoWorkflow(DockerScenario),
    ArgoWorkflowComplete,

    CreateScenarioConfigMap(DockerScenario),
    CreateScenarioJob(DockerScenario),
    WaitForScenarioJob,

    CleanupNamespace,
    MarkUnrunnable(String),
}

impl EventData {
    fn name(&self) -> &'static str {
        match self {
            Self::CreateEnvConfigMap(_, _) => "CreateEnvConfigMap",
            Self::CreateEnvArgoWorkflow(_) => "CreateEnvArgoWorkflow",
            Self::WaitForEnvArgoWorkflow(_) => "WaitForEnvArgoWorkflow",
            Self::ArgoWorkflowComplete => "ArgoWorkflowComplete",
            Self::CreateScenarioConfigMap(_) => "CreateScenarioConfigMap",
            Self::CreateScenarioJob(_) => "CreateScenarioJob",
            Self::WaitForScenarioJob => "WaitForScenarioJob",
            Self::MarkUnrunnable(_) => "MarkUnrunnable",
            Self::CleanupNamespace => "CleanupNamespace",
        }
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
    async fn handle(
        self,
        event_queue: &EventQueue,
        orchestrator_url: &str,
        kubeconfig_secret_name: &str,
        clients: ClusterClients,
    ) -> Result<()> {
        let conn = conn!();

        let res = match self.data {
            EventData::CreateEnvConfigMap(environment, scenario) => {
                provision_environment::create_config_map(
                    self.test_execution.clone(),
                    environment,
                    scenario,
                    clients.clone(),
                    conn,
                )
                .await
            }

            EventData::CreateEnvArgoWorkflow(scenario) => {
                provision_environment::create_workflow(
                    self.test_execution.clone(),
                    scenario,
                    orchestrator_url,
                    kubeconfig_secret_name,
                    clients.clone(),
                    conn,
                )
                .await
            }

            EventData::WaitForEnvArgoWorkflow(scenario) => {
                provision_environment::wait_for_workflow(
                    self.test_execution.clone(),
                    scenario,
                    event_queue.tx(),
                    clients.clone(),
                    conn,
                )
                .await
            }

            EventData::ArgoWorkflowComplete => {
                conn.mark_execution_as_provisioning(&self.test_execution, MSG_ARGO_COMPLETE.into())
                    .await;

                Ok(None)
            }

            EventData::CreateScenarioConfigMap(scenario) => {
                run_scenario::create_config_map(
                    self.test_execution.clone(),
                    scenario,
                    clients,
                    conn,
                )
                .await
            }

            EventData::CreateScenarioJob(scenario) => {
                run_scenario::create_job(
                    self.test_execution.clone(),
                    scenario,
                    orchestrator_url,
                    clients,
                    conn,
                )
                .await
            }

            EventData::WaitForScenarioJob => {
                run_scenario::wait_for_job(
                    self.test_execution.clone(),
                    event_queue.tx(),
                    clients,
                    conn,
                )
                .await
            }

            EventData::CleanupNamespace => {
                let res = cleanup_namespace::try_run(self.test_execution.clone(), clients).await;
                event_queue
                    .mark_execution_complete(self.test_execution.uuid())
                    .await;

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
                    .await
            }
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
