use crate::commands::{
    get_context, load_and_resolve_test_plan_from_github, load_and_resolve_test_plan_from_local,
};
use rep_orchestrator_shared::payload::PreparedPayload;
use rtf_core::variables::Variables;
use tracing::info;

pub async fn write_remote_trigger_payload_to_stdout(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<()> {
    let payload =
        prepare_remote_trigger_payload(test_plan_path, github, git_ref, variables).await?;
    print!("{}", serde_json::to_string_pretty(&payload)?);

    Ok(())
}

pub async fn prepare_remote_trigger_payload(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<PreparedPayload> {
    let ctx = get_context();

    info!("loading and resolving test plan");
    let (test_plan, sources) = if github {
        load_and_resolve_test_plan_from_github(test_plan_path, git_ref, &ctx).await?
    } else {
        load_and_resolve_test_plan_from_local(test_plan_path, &ctx).await?
    };

    let (parsed, vars_file_src) = variables.parse(&ctx)?;

    Ok(PreparedPayload::prepare(test_plan, sources, parsed, vars_file_src, ctx).await?)
}
