use crate::commands::get_context_and_check_outdir;
use futures::future::try_join_all;
use rep_orchestrator_shared::summary::{TestExecutionSummary, TestRunSummary};
use reqwest::Method;
use rtf_config::context::ResolutionContext;
use rtf_integrations::orchestrator::OrchestratorClient;
use std::path::Path;
use tracing::info;
use uuid::Uuid;

const N_PARALLEL_FETCH: usize = 20;

pub async fn pull_run_output(run_id: Uuid, out_dir: &str, force: bool) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_check_outdir(out_dir, force)?;
    let client = OrchestratorClient::new().await?;

    let tr: TestRunSummary = client
        .get_json(&format!("test-run/{run_id}/status"))
        .await?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    ctx.write(
        out_dir.join("run-summary.json"),
        serde_json::to_string_pretty(&tr)?,
    )?;

    let out_dir = ctx.canonicalize_path(&out_dir)?;

    for chunk in tr.executions.chunks(N_PARALLEL_FETCH) {
        let sub_dirs = chunk
            .iter()
            .map(|ex| {
                let sub_dir = out_dir.join(&ex.name);
                ctx.create_dir_all(&sub_dir)?;
                Ok(sub_dir)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        try_join_all(
            chunk
                .iter()
                .zip(sub_dirs.iter())
                .map(|(ex, sub_dir)| write_output_for_execution(ex, sub_dir, &client, &ctx)),
        )
        .await?;
    }

    Ok(())
}

pub async fn pull_execution_output(ex_id: Uuid, out_dir: &str, force: bool) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_check_outdir(out_dir, force)?;
    let client = OrchestratorClient::new().await?;

    let ex: TestExecutionSummary = client
        .get_json(&format!("test-execution/{ex_id}/status"))
        .await?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    ctx.write(
        out_dir.join("execution-summary.json"),
        serde_json::to_string_pretty(&ex)?,
    )?;

    let out_dir = ctx.canonicalize_path(&out_dir)?;

    write_output_for_execution(&ex, &out_dir, &client, &ctx).await
}

async fn write_output_for_execution(
    ex: &TestExecutionSummary,
    out_dir: &Path,
    client: &OrchestratorClient,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("pulling execution log for {}", ex.id);
    let resp = client
        .request(Method::GET, &format!("test-execution/{}/log.txt", ex.id))
        .await?
        .send()
        .await?;

    let log = resp.error_for_status()?.text().await?;

    info!("pulling execution output for {}", ex.id);
    let resp = client
        .request(Method::GET, &format!("test-execution/{}/output.zip", ex.id))
        .await?
        .send()
        .await?;

    let zip_bytes = resp.error_for_status()?.bytes().await?;

    ctx.write(out_dir.join("log.txt"), log)?;
    ctx.write(out_dir.join("output.zip"), zip_bytes)?;

    Ok(())
}
