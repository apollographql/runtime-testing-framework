use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunSummary};
use rtf_integrations::orchestrator::OrchestratorClient;
use uuid::Uuid;

pub async fn execution_status(id: Uuid) -> anyhow::Result<()> {
    eprintln!("Fetching current execution status for {id}...");
    let client = OrchestratorClient::new().await?;
    let summary: TestExecutionSummary = client
        .get_json(&format!("test-execution/{id}/status"))
        .await?;

    println!("{}", serde_json::to_string_pretty(&summary)?);

    Ok(())
}

pub async fn run_status(id: Uuid, with_executions: bool) -> anyhow::Result<()> {
    eprintln!("Fetching current run status for {id}...");
    let client = OrchestratorClient::new().await?;
    let summary: TestRunSummary = client
        .get_json(&format!(
            "test-run/{id}/status?with_executions={with_executions}"
        ))
        .await?;

    println!("{}", serde_json::to_string_pretty(&summary)?);

    Ok(())
}
