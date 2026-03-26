use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use std::path::Path;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{error, warn};

use crate::config::Config;
use crate::db::TestExecution;
use crate::k8s::ClusterClients;

pub mod provision_environment;

#[derive(Debug)]
pub enum EventType {
    ProvisionEnvironment(DockerComposeEnvironment, DockerScenario),
}

#[derive(Debug)]
pub struct Event {
    pub test_execution: TestExecution,
    pub ty: EventType,
}

pub async fn event_loop_task(_etx: UnboundedSender<Event>, mut erx: UnboundedReceiver<Event>) {
    let cfg = Config::get();
    let clients = match ClusterClients::try_new(
        Path::new(&cfg.kubeconfig_path),
        &cfg.mgmt_context,
        &cfg.workload_context,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(%e, "failed to initialise k8s clients, event loop cannot start");
            return;
        }
    };

    while let Some(event) = erx.recv().await {
        match event.ty {
            EventType::ProvisionEnvironment(environment, scenario) => {
                provision_environment::run(event.test_execution, environment, scenario, &clients)
                    .await;
            }
        }
    }

    warn!("event loop channel closed, exiting event loop task");
}
