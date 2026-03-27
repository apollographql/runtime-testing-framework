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
    k8s::ClusterClients,
};
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{error, warn};

pub mod provision_environment;

/// Run as a long lived task. This is an infinite loop that processes [Event]s received on a
/// channel that is shared with the axum server and the event loop's own handler functions.
pub async fn event_loop_task(etx: UnboundedSender<Event>, mut erx: UnboundedReceiver<Event>) {
    let Config {
        kubeconfig_path,
        mgmt_context,
        workload_context,
        ..
    } = Config::get();

    let clients = ClusterClients::try_new(kubeconfig_path, mgmt_context, workload_context)
        .await
        .unwrap_or_else(|e| {
            panic!("failed to initialise k8s clients, event loop cannot start: {e}")
        });

    while let Some(evt) = erx.recv().await {
        let ty_name = evt.data.name();

        if let Err(err) = evt.handle(etx.clone(), clients.clone()).await {
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
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum EventData {
    ProvisionEnvironment(DockerComposeEnvironment, DockerScenario),
    RunScenario(DockerScenario),
    MarkUnrunnable(String),
}

impl EventData {
    fn name(&self) -> &'static str {
        match self {
            Self::ProvisionEnvironment(_, _) => "ProvisionEnvironment",
            Self::RunScenario(_) => "RunScenario",
            Self::MarkUnrunnable(_) => "MarkUnrunnable",
        }
    }
}

/// Raw event data paired with an associated [TestExecution] so we can track the status of the
/// execution as we process it.
#[derive(Debug)]
pub struct Event {
    pub test_execution: TestExecution,
    pub data: EventData,
}

impl Event {
    async fn handle(self, etx: UnboundedSender<Event>, clients: ClusterClients) -> Result<()> {
        let conn = conn!();

        let res = match self.data {
            EventData::MarkUnrunnable(message) => {
                conn.mark_execution_as_unrunnable(&self.test_execution, message)
                    .await;

                Ok(())
            }

            EventData::ProvisionEnvironment(environment, scenario) => {
                provision_environment::try_run(
                    self.test_execution.clone(),
                    environment,
                    scenario,
                    etx.clone(),
                    clients.clone(),
                    conn,
                )
                .await
            }

            EventData::RunScenario(_scenario) => {
                warn!("RunScenario not yet implemented");
                Ok(())
            }
        };

        if let Err(e) = &res {
            conn.mark_execution_as_unrunnable(&self.test_execution, e.to_string())
                .await
        };

        res
    }
}
