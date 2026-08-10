use crate::{
    context::{CliContext, FsError, FsErrorKind},
    orchestrator::Client as OrchestratorClient,
};
use std::{env::temp_dir, fs, process::Command};
use tracing::info;

/// This runs per-pod that needs to make use of file provider output within the environment
/// definition so we don't push statuses back to the orchestrator here, only log.
pub async fn resolve_environment(outdir: &str, ctx: &impl CliContext) -> crate::Result<()> {
    info!("fetching environment configuration");
    let cfg_bytes = ctx.orchestrator_client().fetch_environment_config().await?;

    let cfg_path = temp_dir().join("environment.yaml");
    ctx.write_file(&cfg_path, &cfg_bytes)?;

    fs::create_dir_all(outdir).map_err(|source| FsError {
        path: outdir.into(),
        kind: FsErrorKind::CreateDir,
        source,
    })?;

    ctx.run_shell(Command::new("rtf").args([
        "resolve",
        "environment",
        &cfg_path.to_string_lossy(),
        "--outdir",
        outdir,
        "-vvv",
    ]))
    .await?;

    info!("environment resolved successfully");

    Ok(())
}
