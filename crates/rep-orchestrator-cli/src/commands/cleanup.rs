use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    kubernetes::Client,
};
use anyhow::Context;
use tracing::info;

/// Delete a ConfigMap, typically invoked from the workflow's `on_exit` handler.
///
/// Runs silently on the status timeline so we don't clobber the execution's terminal status
/// with a spurious update from a post-termination cleanup step. Progress is traced to pod
/// logs via `tracing::info!`. On failure, the error propagates through the CLI's top-level
/// error handler, which posts `Unrunnable` with the error message — so cleanup failures
/// remain visible in both the status timeline and the pod logs.
pub async fn cleanup(configmap: &str, namespace: &str, ctx: &impl CliContext) -> CliResult<()> {
    info!("Cleaning up ConfigMap '{configmap}' in namespace '{namespace}'...");

    ctx.kube_client()
        .delete_configmap(configmap, namespace)
        .await
        .context("Failed to delete configmap")
        .map_err(CliError::unrunnable)?;

    info!("ConfigMap deleted.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::mocks::MockContext;
    use crate::kubernetes::mocks::{KubeCall, MockClient as MockKubeClient};
    use rep_orchestrator_shared::status::Status;

    #[tokio::test]
    async fn success_posts_no_status_updates() {
        let ctx = MockContext::default();
        cleanup("test-cm", "test-ns", &ctx).await.unwrap();

        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()));
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
    async fn kube_failure_returns_unrunnable_error_without_posting_status() {
        let ctx = MockContext {
            kube_client: MockKubeClient::failing(),
            ..Default::default()
        };
        let err = cleanup("test-cm", "test-ns", &ctx).await.unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);

        // The CLI's top-level error handler is what posts Unrunnable on error; cleanup
        // itself must not post anything.
        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()));
    }
}
