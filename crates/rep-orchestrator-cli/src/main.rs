mod cli;
mod commands;

use clap::Parser;
use cli::{Args, Command};
use std::process::exit;
use tracing::error;

const LOG_LEVEL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_LOG";

#[tokio::main]
async fn main() {
    let Args { command, verbose } = Args::parse();

    // Unlike rtf-cli, we start at INFO as our default log level
    if let Err(e) = rtf_cli_shared::init_logging(LOG_LEVEL_ENV_VAR, verbose + 1) {
        error!("unable to initialise logging: {e}");
        exit(1);
    };

    let res = match command {
        Command::CreateNamespace {
            namespace,
            kubeconfig: kubeconfig_path,
        } => commands::create_namespace(&namespace, &kubeconfig_path).await,

        Command::CreatePullSecret {
            namespace,
            kubeconfig: kubeconfig_path,
            docker_config: docker_config_path,
        } => commands::create_pull_secret(&namespace, &kubeconfig_path, &docker_config_path).await,

        Command::DeployEnvironment {
            namespace,
            kubeconfig: kubeconfig_path,
            environment: environment_path,
            timeout,
        } => {
            commands::deploy_environment(&namespace, &kubeconfig_path, &environment_path, timeout)
                .await
        }

        Command::Cleanup {
            configmap,
            namespace,
        } => commands::cleanup(&configmap, &namespace).await,
    };

    if let Err(e) = res {
        error!("{e}");
        exit(1);
    }
}
