use crate::commands::{get_context, plumbing::prepare_remote_trigger_payload};
use rtf_core::variables::Variables;
use rtf_integrations::orchestrator::OrchestratorClient;
use rtf_orchestrator_shared::{
    payload::{KnownTestPlanUuidPayload, TriggerPayload},
    summary::TestRunSummary,
};
use uuid::Uuid;

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
    let client = OrchestratorClient::new_from_env().await?;

    println!("Triggering test run...\n");
    let summary: TestRunSummary = client.post_json("test-run/trigger", &payload).await?;
    let id = summary.id;
    println!("Test run id: {id}\n");

    let url = format!("https://api.rtf.apollographql.com/ui/run/{id}");
    println!("View test run status: {url}");

    Ok(())
}

pub async fn remote_run_known(
    test_plan_uuid: Uuid,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<()> {
    println!("Triggering test run for test plan with ID={test_plan_uuid}...");
    let client = OrchestratorClient::new_from_env().await?;

    let ctx = get_context();
    let (parsed_variables, _) = variables.parse(&ctx)?;
    let flat = parsed_variables.as_flat();
    let payload = TriggerPayload::KnownTestPlanUuid(KnownTestPlanUuidPayload {
        test_plan_uuid,
        git_ref,
        variables: (!flat.is_empty()).then_some(flat),
    });

    println!("Triggering test run...\n");
    let summary: TestRunSummary = client.post_json("test-run/trigger", &payload).await?;
    let id = summary.id;
    println!("Test run id: {id}\n");

    let url = format!("https://api.rtf.apollographql.com/ui/run/{id}");
    println!("View test run status: {url}");

    Ok(())
}
