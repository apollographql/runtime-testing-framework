use crate::cli::Args;
use anyhow::bail;
use clap::CommandFactory;
use clap_complete::{Shell, generate};
use std::io::stdout;

pub fn generate_shell_completions(shell: Option<Shell>) -> anyhow::Result<()> {
    let shell = match shell.or_else(Shell::from_env) {
        Some(shell) => shell,
        None => bail!("Unable to determine current shell, please specify using the --shell flag"),
    };

    let mut cmd = Args::command();
    let name = cmd.get_name().to_string();

    generate(shell, &mut cmd, name, &mut stdout());

    Ok(())
}
