use crate::{context::CliContext, orchestrator::Client as OrchestratorClient};
use std::{env::temp_dir, os::unix::fs::PermissionsExt, path::Path, process::Command};
use tracing::info;

/// This runs per-pod that needs to make use of file provider output within the environment
/// definition so we don't push statuses back to the orchestrator here, only log.
pub async fn resolve_environment(outdir: &str, ctx: &impl CliContext) -> crate::Result<()> {
    info!("fetching environment configuration");
    let cfg_bytes = ctx.orchestrator_client().fetch_environment_config().await?;

    let cfg_path = temp_dir().join("environment.yaml");
    ctx.write_file(&cfg_path, &cfg_bytes)?;
    ctx.create_dir_all(Path::new(outdir))?;
    ctx.run_shell(Command::new("rtf").args([
        "resolve",
        "environment",
        &cfg_path.to_string_lossy(),
        "--outdir",
        outdir,
        "-vvv",
    ]))
    .await?;

    // Ensure that all files/directories we've written out are world writable and that directories
    // can be traversed. We don't get this for free as we can't guarantee that this CLI is running
    // under the same UID that the user provided container consuming the files is using.
    for entry in ctx.list_entries_under(Path::new(outdir))? {
        let mode = ctx.permissions(&entry.path)?.mode();
        let additional = if entry.is_dir {
            // write + execute (traverse) for owner/group/other
            0o333
        } else {
            // write for owner/group/other
            0o222
        };

        ctx.set_permissions_mode(&entry.path, mode | additional)?;
    }

    info!("environment resolved successfully");

    Ok(())
}
