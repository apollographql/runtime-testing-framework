use crate::k8s::ClusterClients;
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use std::{env, path::Path};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{error, info};
use uuid::Uuid;

mod cleanup_environment;
mod provision_environment;
mod run_scenario;

#[derive(Debug)]
pub struct Event {
    pub execution_id: Uuid,
    pub ty: EventType,
}

#[derive(Debug)]
pub enum EventType {
    ProvisionEnvironment(DockerComposeEnvironment, DockerScenario),
    RunScenario(DockerScenario),
    CleanupNamespace,
}

pub async fn event_loop_task(tx: UnboundedSender<Event>, mut rx: UnboundedReceiver<Event>) {
    // FIXME: handle getting the kubeconfig path correctly here
    let clients = match ClusterClients::try_new(
        Path::new(&env::var("KUBECONFIG_PATH").expect("no kubeconfig path")),
        "kind-rtf-mgmt",
        "kind-rtf-workload",
    )
    .await
    {
        Ok(clients) => clients,
        Err(error) => {
            error!(%error, "unable to create k8s clients");
            return;
        }
    };

    loop {
        let evt = match rx.recv().await {
            Some(evt) => evt,
            None => {
                info!("Event loop channel closed. Exiting event loop task");
                return;
            }
        };

        if let Err(e) = handle_event(evt, &tx, &clients).await {
            error!(%e, "error processing event");
        }
    }
}

async fn handle_event(
    evt: Event,
    tx: &UnboundedSender<Event>,
    clients: &ClusterClients,
) -> anyhow::Result<()> {
    match evt.ty {
        EventType::ProvisionEnvironment(environment, scenario) => {
            provision_environment::run(evt.execution_id, environment, scenario, tx, clients).await
        }
        EventType::RunScenario(scenario) => {
            run_scenario::run(evt.execution_id, scenario, tx, clients).await
        }
        EventType::CleanupNamespace => cleanup_environment::run(evt.execution_id, clients).await,
    }
}
