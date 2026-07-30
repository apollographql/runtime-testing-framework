use rtf_integrations::orchestrator::OrchestratorClient;
use uuid::Uuid;

pub async fn execution_log(id: Uuid) -> anyhow::Result<()> {
    eprintln!("Fetching current execution status for {id}...");
    let client = OrchestratorClient::new_from_env().await?;
    let log = client
        .get_text(&format!("test-execution/{id}/log.txt"))
        .await?;

    println!("{log}");

    Ok(())
}
