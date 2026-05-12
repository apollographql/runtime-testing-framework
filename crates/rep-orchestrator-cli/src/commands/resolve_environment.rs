use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    orchestrator::Client as OrchestratorClient,
};
use anyhow::Context;
use std::{env::temp_dir, fs, process::Command};
use tracing::info;

/// This runs per-pod that needs to make use of file provider output within the environment
/// definition so we don't push statuses back to the orchestrator here, only log.
pub async fn resolve_environment(outdir: &str, ctx: &impl CliContext) -> CliResult<()> {
    info!("Fetching environment configuration...");
    let cfg_bytes = ctx
        .orchestrator_client()
        .fetch_environment_config()
        .await
        .map_err(CliError::unrunnable)?;

    let cfg_path = temp_dir().join("environment.yaml");
    ctx.write_file(&cfg_path, &cfg_bytes)?;

    fs::create_dir_all(outdir)
        .context(format!(
            "Failed to create provider output directory ({outdir})"
        ))
        .map_err(CliError::unrunnable)?;

    ctx.run_shell(Command::new("rtf").args([
        "resolve",
        "environment",
        &cfg_path.to_string_lossy(),
        "--outdir",
        outdir,
        "-vvv",
    ]))
    .await?;

    info!("Environment resolved successfully");

    Ok(())
}
