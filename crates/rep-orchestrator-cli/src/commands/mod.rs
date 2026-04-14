mod cleanup;
mod create_namespace;
mod create_pull_secret;
mod deploy_environment;

pub use cleanup::cleanup;
pub use create_namespace::create_namespace;
pub use create_pull_secret::create_pull_secret;
pub use deploy_environment::deploy_environment;

use crate::error::{CliError, CliResult};
use anyhow::Context;
use kube::{
    Client, Config,
    config::{KubeConfigOptions, Kubeconfig},
};
use std::path::Path;

const MANAGER_NAME: &str = "rep-orchestrator-cli";

/// Build a kube client from a kubeconfig file path.
// TODO: abstract this into Context. We may need to newtype Client to make it actually useful for testing.
async fn client_from_kubeconfig(kubeconfig_path: Option<&Path>) -> CliResult<Client> {
    match kubeconfig_path {
        Some(path) => {
            let kfg = Kubeconfig::read_from(path)
                .with_context(|| format!("failed to read kubeconfig from {}", path.display()))
                .map_err(CliError::unrunnable)?;

            let config = Config::from_custom_kubeconfig(kfg, &KubeConfigOptions::default())
                .await
                .context("failed to build kube config")
                .map_err(CliError::unrunnable)?;

            Client::try_from(config)
        }
        None => Client::try_default().await,
    }
    .context("failed to create kube client from in-cluster config")
    .map_err(CliError::unrunnable)
}
