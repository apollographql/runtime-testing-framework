use crate::{
    commands::client_from_kubeconfig,
    error::{CliError, CliResult},
};
use anyhow::Context;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{Api, api::DeleteParams};
use tracing::info;

pub async fn cleanup(configmap: &str, namespace: &str) -> CliResult<()> {
    info!("Cleaning up ConfigMap '{configmap}'...");

    let client = client_from_kubeconfig(None).await?;

    let api: Api<ConfigMap> = Api::namespaced(client, namespace);
    match api.delete(configmap, &DeleteParams::default()).await {
        Ok(_) => info!("ConfigMap deleted."),
        Err(e) => {
            return Err(e)
                .context("Failed to delete configmap")
                .map_err(CliError::unrunnable);
        }
    }
    info!("Cleanup complete.");

    Ok(())
}
