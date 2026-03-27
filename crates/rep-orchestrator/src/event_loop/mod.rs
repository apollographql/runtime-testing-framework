use crate::{config::Config, db::TestExecution, k8s::ClusterClients};
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{error, warn};

pub mod provision_environment;

#[derive(Debug)]
pub enum EventType {
    ProvisionEnvironment(DockerComposeEnvironment, DockerScenario),
    RunScenario(DockerScenario),
}

impl EventType {
    fn name(&self) -> &'static str {
        match self {
            Self::ProvisionEnvironment(_, _) => "ProvisionEnvironment",
            Self::RunScenario(_) => "RunScenario",
        }
    }
}

#[derive(Debug)]
pub struct Event {
    pub test_execution: TestExecution,
    pub ty: EventType,
}

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

    while let Some(Event { test_execution, ty }) = erx.recv().await {
        let ty_name = ty.name();

        let res = match ty {
            EventType::ProvisionEnvironment(environment, scenario) => {
                provision_environment::run(
                    test_execution,
                    environment,
                    scenario,
                    etx.clone(),
                    clients.clone(),
                )
                .await
            }

            EventType::RunScenario(_scenario) => {
                warn!("RunScenario not yet implemented");
                Ok(())
            }
        };

        if let Err(err) = res {
            error!(%err, ty=%ty_name, "Error handling event");
        }
    }

    warn!("event loop channel closed, exiting event loop task");
}
