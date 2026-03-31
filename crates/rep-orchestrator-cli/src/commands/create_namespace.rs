use crate::{
    commands::{MANAGER_NAME, client_from_kubeconfig},
    error::{CliResult, ToCliResult},
};
use anyhow::Context;
use k8s_openapi::api::core::v1::Namespace;
use kube::{
    Api,
    api::{ObjectMeta, Patch, PatchParams},
};
use std::path::Path;
use tracing::info;

pub async fn create_namespace(namespace: &str, kubeconfig_path: &Path) -> CliResult<()> {
    info!("Creating namespace '{namespace}' in workload cluster...");

    let client = client_from_kubeconfig(Some(kubeconfig_path)).await?;
    let api: Api<Namespace> = Api::all(client);

    let ns = Namespace {
        metadata: ObjectMeta {
            name: Some(namespace.to_owned()),
            ..Default::default()
        },
        ..Default::default()
    };

    api.patch(
        namespace,
        &PatchParams::apply(MANAGER_NAME),
        &Patch::Apply(&ns),
    )
    .await
    .context("Failed to create kube namespace.")
    .err_unrunnable()?;
    info!("Namespace created successfully.");

    Ok(())
}
