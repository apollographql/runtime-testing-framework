use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{info, warn};

use crate::db::TestExecution;

#[derive(Debug)]
pub enum EventType {
    ProvisionEnvironment(DockerComposeEnvironment, DockerScenario),
}

#[derive(Debug)]
pub struct Event {
    pub test_execution: TestExecution,
    pub ty: EventType,
}

pub async fn event_loop_task(mut erx: UnboundedReceiver<Event>) {
    while let Some(event) = erx.recv().await {
        match event.ty {
            EventType::ProvisionEnvironment(environment, scenario) => {
                // This is just a stubbed version of the event loop task. Future PRs will need to implement the full task
                info!(
                    execution_id = %event.test_execution.uuid(),
                    environment = ?environment,
                    scenario = ?scenario,
                    "ProvisionEnvironment received - logic to do something with it not yet implemented"
                );
            }
        }
    }

    warn!("event loop channel closed, exiting event loop task");
}
