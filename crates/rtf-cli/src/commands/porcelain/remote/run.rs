use crate::commands::plumbing::prepare_remote_trigger_payload;
use rep_orchestrator_shared::summary::TestRunSummary;
use rtf_core::variables::Variables;
use rtf_integrations::orchestrator::OrchestratorClient;

pub async fn remote_run(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<()> {
    println!("Preparing trigger payload for {test_plan_path}...");
    match (github, git_ref.as_ref()) {
        (true, Some(s)) => println!("  pulling from GitHub using git ref {s:?}"),
        (true, None) => println!("  pulling from GitHub"),
        _ => (),
    }

    let payload =
        prepare_remote_trigger_payload(test_plan_path, github, git_ref, variables).await?;
    let client = OrchestratorClient::new().await?;

    println!("Triggering test run...\n");
    let summary: TestRunSummary = client.post_json("test-run/trigger", &payload).await?;
    let id = summary.id;
    println!("Test run id: {id}\n");

    let url = format!("https://api.rtf.apollographql.com/ui/run/{id}");
    println!("View test run status: {url}");

    Ok(())
}
