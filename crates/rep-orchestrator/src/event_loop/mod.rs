use crate::db::TestExecution;
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::warn;

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

pub async fn event_loop_task(mut erx: UnboundedReceiver<Event>) {
    while let Some(event) = erx.recv().await {
        match event.ty {
            EventType::ProvisionEnvironment(environment, scenario) => {
                provision_environment::run(event.test_execution, environment, scenario).await;
            }
        }
    }

    warn!("event loop channel closed, exiting event loop task");
}
