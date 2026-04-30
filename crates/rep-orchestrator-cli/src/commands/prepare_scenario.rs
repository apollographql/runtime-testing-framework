use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    info_status,
    orchestrator::Client as OrchestratorClient,
};
use rep_orchestrator_shared::status::Status;
use std::{
    env::temp_dir,
    path::{Path, PathBuf},
    process::Command,
};

// On exit, the wrapper touches a sentinel file so the output-collector container (which runs
// in parallel with the scenario-runner) can detect completion via polling. The real scenario
// exit code is written to __EXIT_STATUS_FILE__ for the output-collector to read.
const RUN_SCRIPT_TEMPLATE: &str = r#"#!/bin/sh
set -ex

trap 'touch __EXIT_SENTINEL__' EXIT

mkdir -p __OUTPUT_DIR__

{
  (
    set -ex
__SCENARIO_ENV__
    export OUTDIR=__OUTPUT_DIR__
    export RTF_OUTPUT="$OUTDIR/RTF_OUTPUT"

__SCENARIO_COMMAND__
  ) 2>&1
  echo $? > __EXIT_STATUS_FILE__
} | tee __OUTPUT_DIR__/output.log
"#;

/// Resolve the scenario config into `<shared_dir>/providers` and write a `run.sh` script into
/// `<shared_dir>` that inlines the resolved env vars and invokes the user's scenario command.
pub async fn prepare_scenario(
    shared_dir: &Path,
    scenario_command: &str,
    ctx: &impl CliContext,
) -> CliResult<()> {
    let paths = SharedPaths::new(shared_dir);

    info_status!(
        ctx,
        Status::Provisioning,
        "Fetching scenario configuration..."
    )?;

    let cfg_bytes = ctx
        .orchestrator_client()
        .fetch_scenario_config()
        .await
        .map_err(CliError::unrunnable)?;

    let workdir_path = temp_dir().join("rtf-work");
    let cfg_path = &workdir_path.join("scenario.yaml");
    ctx.write_file(cfg_path, &cfg_bytes)?;

    info_status!(ctx, Status::Provisioning, "Resolving scenario...")?;
    ctx.run_shell(
        Command::new("rtf")
            .args(["resolve", "scenario"])
            .arg(cfg_path)
            .args(["--outdir", &paths.providers_dir.to_string_lossy()]),
        Status::Provisioning,
    )
    .await?;

    let scenario_env = ctx.read_file_to_string(&paths.providers_dir.join("scenario.env"))?;

    let run_sh = render_run_script(&paths, &scenario_env, scenario_command);
    ctx.write_file(&paths.run_script, run_sh.as_bytes())?;
    ctx.set_permissions_mode(&paths.run_script, 0o777)?;

    info_status!(
        ctx,
        Status::Running,
        "Scenario resolved, run execution beginning..."
    )?;

    Ok(())
}

struct SharedPaths {
    providers_dir: PathBuf,
    output_dir: PathBuf,
    run_script: PathBuf,
    exit_sentinel: PathBuf,
    exit_status_file: PathBuf,
}

impl SharedPaths {
    fn new(shared_dir: &Path) -> Self {
        Self {
            providers_dir: shared_dir.join("providers"),
            output_dir: shared_dir.join("output"),
            run_script: shared_dir.join("run.sh"),
            exit_sentinel: shared_dir.join("scenario-exited"),
            exit_status_file: shared_dir.join("scenario_exit_status"),
        }
    }
}

fn render_run_script(paths: &SharedPaths, scenario_env: &str, command: &str) -> String {
    let indented_env = scenario_env
        .lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n");

    let indented_command = command
        .lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n");

    RUN_SCRIPT_TEMPLATE
        .replace("__EXIT_SENTINEL__", &paths.exit_sentinel.to_string_lossy())
        .replace("__OUTPUT_DIR__", &paths.output_dir.to_string_lossy())
        .replace("__SCENARIO_ENV__", &indented_env)
        .replace("__SCENARIO_COMMAND__", &indented_command)
        .replace(
            "__EXIT_STATUS_FILE__",
            &paths.exit_status_file.to_string_lossy(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> SharedPaths {
        SharedPaths::new(&PathBuf::from("/shared"))
    }

    #[test]
    fn run_script_substitutes_all_placeholders() {
        let rendered = render_run_script(
            &paths(),
            "export FOO=bar\nexport BAZ=qux",
            "echo hi > $RTF_OUTPUT",
        );

        assert!(
            !rendered.contains("__"),
            "unsubstituted placeholder in:\n{rendered}"
        );
        assert!(rendered.contains("trap 'touch /shared/scenario-exited' EXIT"));
        assert!(rendered.contains("mkdir -p /shared/output"));
        assert!(rendered.contains("echo $? > /shared/scenario_exit_status"));
        assert!(rendered.contains("tee /shared/output/output.log"));
    }

    #[test]
    fn run_script_inlines_scenario_env_and_command_with_indent() {
        let rendered = render_run_script(
            &paths(),
            "export FOO=bar\nexport BAZ=qux",
            "first_line\nsecond_line",
        );

        // Both env and command should land inside the subshell with 4-space indent
        assert!(rendered.contains("    export FOO=bar\n    export BAZ=qux"));
        assert!(rendered.contains("    first_line\n    second_line"));
    }
}
