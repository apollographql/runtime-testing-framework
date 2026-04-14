use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    info_status,
    kubernetes::Client,
};
use anyhow::Context;
use rep_orchestrator_shared::status::Status;
use std::path::Path;

pub async fn create_pull_secret(
    namespace: &str,
    docker_config_path: &Path,
    ctx: &impl CliContext,
) -> CliResult<()> {
    info_status!(
        ctx,
        Status::Provisioning,
        "Creating image pull secret in namespace '{namespace}'..."
    )?;

    let docker_config_json = std::fs::read(docker_config_path)
        .with_context(|| {
            format!(
                "Failed to read docker config from {}",
                docker_config_path.display()
            )
        })
        .map_err(CliError::unrunnable)?;

    let client = ctx.kube_client();

    client
        .create_pull_secret(namespace, docker_config_json)
        .await
        .context("Failed to create pull secret")
        .map_err(CliError::unrunnable)?;

    info_status!(
        ctx,
        Status::Provisioning,
        "Patching default service account..."
    )?;

    client
        .patch_default_service_account(namespace)
        .await
        .context("Failed to patch default service account")
        .map_err(CliError::unrunnable)?;

    info_status!(
        ctx,
        Status::Provisioning,
        "Pull secret created successfully."
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::mocks::MockContext;
    use crate::kubernetes::mocks::{KubeCall, MockClient as MockKubeClient};
    use crate::orchestrator::mocks::MockClient as MockOrchestrator;
    use rep_orchestrator_shared::status::Status;
    use std::path::PathBuf;

    #[tokio::test]
    async fn full_status_transition() {
        let ctx = MockContext::default();
        create_pull_secret("test-ns", &PathBuf::from("/dev/null"), &ctx)
            .await
            .unwrap();

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 3);
            assert_eq!(updates[0].status, Status::Provisioning);
            assert_eq!(updates[1].status, Status::Provisioning);
            assert_eq!(updates[2].status, Status::Provisioning);
        });
    }

    #[tokio::test]
    async fn calls_kube_operations_in_order() {
        let ctx = MockContext::default();
        create_pull_secret("my-ns", &PathBuf::from("/dev/null"), &ctx)
            .await
            .unwrap();

        ctx.kube_client.read_calls(|calls| {
            assert_eq!(
                calls,
                &[
                    KubeCall::ApplyPullSecret {
                        namespace: "my-ns".to_owned(),
                    },
                    KubeCall::PatchDefaultServiceAccount {
                        namespace: "my-ns".to_owned(),
                    },
                ]
            );
        });
    }

    #[tokio::test]
    async fn kube_failure_stops_after_initial_status() {
        let ctx = MockContext {
            kube_client: MockKubeClient::failing(),
            ..Default::default()
        };
        let err = create_pull_secret("test-ns", &PathBuf::from("/dev/null"), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].status, Status::Provisioning);
        });
    }

    #[tokio::test]
    async fn aborts_on_status_update_failure() {
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::failing(),
            ..Default::default()
        };
        let err = create_pull_secret("test-ns", &PathBuf::from("/dev/null"), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }
}
