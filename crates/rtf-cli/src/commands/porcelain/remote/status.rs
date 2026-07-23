use rep_orchestrator_shared::summary::TestExecutionSummary;
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
