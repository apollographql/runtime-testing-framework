use anyhow::bail;
use clap::Parser;
use rtf_cli_shared::init_logging;
use rtf_orchestrator_cli::{Args, EnvironmentContext, run_command};
use rustls::crypto::aws_lc_rs;
use std::io::stdout;

const LOG_LEVEL_ENV_VAR: &str = "APOLLO_RTF_ORCHESTRATOR_LOG";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args { command, verbose } = Args::parse();

    // Unlike the main rtf-cli, we start at INFO as our default log level and target stdout instead
    // of stderr in order to not have logs be tagged as errors in the GCP logs view.
    if let Err(e) = init_logging(LOG_LEVEL_ENV_VAR, verbose + 1, stdout) {
        bail!("unable to initialise logging: {e}");
    };

    if aws_lc_rs::default_provider().install_default().is_err() {
        panic!("unable to install default crypto provider");
    }

    let ctx = match EnvironmentContext::from_environment(command.kubeconfig()).await {
        Err(e) => bail!("unable to initialize RTF Orchestrator CLI: {e}"),
        Ok(context) => context,
    };

    run_command(command, &ctx).await
}
