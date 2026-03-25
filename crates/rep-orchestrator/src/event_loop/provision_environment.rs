use rtf_config::formats::{DockerComposeEnvironment, DockerScenario};
use tracing::info;

use crate::db::TestExecution;

pub async fn run(
    test_execution: TestExecution,
    environment: DockerComposeEnvironment,
    scenario: DockerScenario,
) {
    // This is just a stubbed version of the event loop task. Future PRs will need to implement the full task
    info!(
        execution_id = %test_execution.uuid(),
        environment = ?environment,
        scenario = ?scenario,
        "ProvisionEnvironment received - logic to do something with it not yet implemented"
    );
}
