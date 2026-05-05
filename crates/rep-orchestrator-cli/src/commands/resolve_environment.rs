use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    orchestrator::Client as OrchestratorClient,
};
use rep_orchestrator_shared::status::Status;
use std::{env::temp_dir, process::Command};

pub async fn resolve_environment(outdir: &str, ctx: &impl CliContext) -> CliResult<()> {
    let cfg_bytes = ctx
        .orchestrator_client()
        .fetch_environment_config()
        .await
        .map_err(CliError::unrunnable)?;

    let cfg_path = temp_dir().join("environment.yaml");
    ctx.write_file(&cfg_path, &cfg_bytes)?;

    ctx.run_shell(
        Command::new("rtf").args([
            "resolve",
            "environment",
            &cfg_path.to_string_lossy(),
            "--outdir",
            outdir,
            // need to force in order to get rid of lost+found dirs coming from fsck
            "--force",
            "-vvv",
        ]),
        Status::Provisioning,
    )
    .await?;

    Ok(())
}
