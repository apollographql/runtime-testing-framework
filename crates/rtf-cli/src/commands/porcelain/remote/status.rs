use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunSummary};
use reqwest::Url;
use rtf_integrations::orchestrator::OrchestratorClient;
use uuid::Uuid;

pub async fn execution_status(id: Uuid, orchestrator_url: Option<Url>) -> anyhow::Result<()> {
    eprintln!("Fetching current execution status for {id}...");
    let client = OrchestratorClient::new(orchestrator_url).await?;
    let summary: TestExecutionSummary = client
        .get_json(&format!("test-execution/{id}/status"))
        .await?;

    println!("{}", serde_json::to_string_pretty(&summary)?);

    Ok(())
}

pub async fn run_status(
    id: Uuid,
    with_executions: bool,
    orchestrator_url: Option<Url>,
) -> anyhow::Result<()> {
    eprintln!("Fetching current run status for {id}...");
    let client = OrchestratorClient::new(orchestrator_url).await?;
    let summary: TestRunSummary = client
        .get_json(&format!(
            "test-run/{id}/status?with_executions={with_executions}"
        ))
        .await?;

    println!("{}", serde_json::to_string_pretty(&summary)?);

    Ok(())
}
