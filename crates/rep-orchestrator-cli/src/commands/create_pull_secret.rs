use crate::commands::{MANAGER_NAME, client_from_kubeconfig};
use anyhow::Context;
use k8s_openapi::api::core::v1::{Secret, ServiceAccount};
use kube::{
    Api,
    api::{ObjectMeta, Patch, PatchParams},
};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use tracing::info;

pub async fn create_pull_secret(
    namespace: &str,
    kubeconfig_path: &Path,
    docker_config_path: &Path,
) -> anyhow::Result<()> {
    info!("Creating image pull secret in namespace '{namespace}'...");

    let client = client_from_kubeconfig(kubeconfig_path).await?;

    let docker_config_json = std::fs::read(docker_config_path).with_context(|| {
        format!(
            "Failed to read docker config from {}",
            docker_config_path.display()
        )
    })?;

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some("gcr-secret".to_owned()),
            namespace: Some(namespace.to_owned()),
            ..Default::default()
        },
        type_: Some("kubernetes.io/dockerconfigjson".to_owned()),
        data: Some(BTreeMap::from([(
            ".dockerconfigjson".to_owned(),
            k8s_openapi::ByteString(docker_config_json),
        )])),
        ..Default::default()
    };

    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    secrets
        .patch(
            "gcr-secret",
            &PatchParams::apply(MANAGER_NAME),
            &Patch::Apply(&secret),
        )
        .await
        .context("Failed to create pull secret")?;

    info!("Patching default service account...");
    let sa_api: Api<ServiceAccount> = Api::namespaced(client, namespace);
    let patch = json!({
        "imagePullSecrets": [{"name": "gcr-secret"}]
    });
    sa_api
        .patch("default", &PatchParams::default(), &Patch::Strategic(patch))
        .await
        .context("Failed to patch default service account")?;
    info!("Pull secret created successfully.");

    Ok(())
}
