use crate::commands::client_from_kubeconfig;
use anyhow::Context;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{Api, api::DeleteParams};
use std::path::Path;
use tracing::info;

pub async fn cleanup(
    configmap: &str,
    namespace: &str,
    kubeconfig_path: &Path,
) -> anyhow::Result<()> {
    info!("Cleaning up ConfigMap '{configmap}'...");

    let client = client_from_kubeconfig(kubeconfig_path)
        .await
        .context("Failed to create kube client")?;

    let api: Api<ConfigMap> = Api::namespaced(client, namespace);
    match api.delete(configmap, &DeleteParams::default()).await {
        Ok(_) => info!("ConfigMap deleted."),
        Err(e) => return Err(e).context("Failed to delete configmap"),
    }
    info!("Cleanup complete.");

    Ok(())
}
