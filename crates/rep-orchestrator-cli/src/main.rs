use anyhow::bail;
use clap::Parser;
use rep_orchestrator_cli::{Args, EnvironmentContext, run_command};
use rustls::crypto::aws_lc_rs;

const LOG_LEVEL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_LOG";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args { command, verbose } = Args::parse();

    // Unlike rtf-cli, we start at INFO as our default log level
    if let Err(e) = rtf_cli_shared::init_logging(LOG_LEVEL_ENV_VAR, verbose + 1) {
        bail!("unable to initialise logging: {e}");
    };

    if aws_lc_rs::default_provider().install_default().is_err() {
        panic!("unable to install default crypto provider");
    }

    let ctx = match EnvironmentContext::from_environment(command.kubeconfig()).await {
        Err(e) => {
            bail!("unable to initialize REP Orchestrator CLI: {e}");
        }
        Ok(context) => context,
    };

    run_command(command, &ctx).await
}
