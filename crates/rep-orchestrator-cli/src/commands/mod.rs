mod cleanup;
mod create_namespace;
mod create_pull_secret;
mod deploy_environment;

pub use cleanup::cleanup;
pub use create_namespace::create_namespace;
pub use create_pull_secret::create_pull_secret;
pub use deploy_environment::deploy_environment;

use anyhow::{Context, bail};
use kube::{
    Client, Config,
    config::{KubeConfigOptions, Kubeconfig},
};
use std::{path::Path, process::Command};
use tracing::info;

const MANAGER_NAME: &str = "rep-orchestrator-cli";

/// Build a kube client from a kubeconfig file path.
async fn client_from_kubeconfig(kubeconfig_path: Option<&Path>) -> anyhow::Result<Client> {
    match kubeconfig_path {
        Some(path) => {
            let kfg = Kubeconfig::read_from(path)
                .with_context(|| format!("failed to read kubeconfig from {}", path.display()))?;

            let config = Config::from_custom_kubeconfig(kfg, &KubeConfigOptions::default())
                .await
                .context("failed to build kube config")?;

            Client::try_from(config)
        }
        None => Client::try_default().await,
    }
    .context("failed to create kube client from in-cluster config")
}

/// Run a shell command, logging it and returning an error if it fails.
fn run_shell(cmd: &mut Command) -> anyhow::Result<()> {
    info!("running: {cmd:?}");
    let status = cmd.status().context("failed to execute command")?;
    if !status.success() {
        bail!("command exited with {status}");
    }
    Ok(())
}
