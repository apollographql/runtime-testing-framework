use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use uuid::Uuid;

#[derive(Debug)]
pub struct Event {
    pub execution_id: Uuid,
    pub ty: EventType,
}

#[derive(Debug)]
pub enum EventType {
    ProvisionNamespace(DockerComposeEnvironment, DockerScenario),
    WaitForNamespace(DockerScenario),
    RunScenario(DockerScenario),
    WaitForScenario,
    CleanupNamespace,
}
