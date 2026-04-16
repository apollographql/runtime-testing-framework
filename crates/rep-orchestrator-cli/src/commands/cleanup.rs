use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    info_status,
    kubernetes::Client,
};
use anyhow::Context;
use rep_orchestrator_shared::status::Status;

pub async fn cleanup(configmap: &str, namespace: &str, ctx: &impl CliContext) -> CliResult<()> {
    info_status!(
        ctx,
        Status::Successful,
        "Cleaning up ConfigMap '{configmap}'..."
    )?;

    ctx.kube_client()
        .delete_configmap(configmap, namespace)
        .await
        .context("Failed to delete configmap")
        .map_err(CliError::unrunnable)?;

    info_status!(ctx, Status::Successful, "ConfigMap deleted.")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::mocks::MockContext;
    use crate::kubernetes::mocks::{KubeCall, MockClient as MockKubeClient};
    use crate::orchestrator::mocks::MockClient as MockOrchestrator;
    use rep_orchestrator_shared::status::Status;

    #[tokio::test]
    async fn full_status_transition() {
        let ctx = MockContext::default();
        cleanup("test-cm", "test-ns", &ctx).await.unwrap();

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[0].status, Status::Successful);
            assert_eq!(updates[1].status, Status::Successful);
        });
    }

    #[tokio::test]
    async fn calls_delete_configmap_with_correct_args() {
        let ctx = MockContext::default();
        cleanup("my-cm", "my-ns", &ctx).await.unwrap();

        ctx.kube_client.read_calls(|calls| {
            assert_eq!(
                calls,
                &[KubeCall::DeleteConfigMap {
                    name: "my-cm".to_owned(),
                    namespace: "my-ns".to_owned(),
                }]
            );
        });
    }

    #[tokio::test]
    async fn status_message_includes_configmap_name() {
        let ctx = MockContext::default();
        cleanup("my-configmap", "test-ns", &ctx).await.unwrap();

        ctx.orchestrator_client().read_updates(|updates| {
            let msg = updates[0].message.as_deref().unwrap();
            assert!(
                msg.contains("my-configmap"),
                "expected configmap name in message: {msg}"
            );
        });
    }

    #[tokio::test]
    async fn kube_failure_stops_after_initial_status() {
        let ctx = MockContext {
            kube_client: MockKubeClient::failing(),
            ..Default::default()
        };
        let err = cleanup("test-cm", "test-ns", &ctx).await.unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].status, Status::Successful);
        });
    }

    #[tokio::test]
    async fn aborts_on_status_update_failure() {
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::failing(),
            ..Default::default()
        };
        let err = cleanup("test-cm", "test-ns", &ctx).await.unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }
}
