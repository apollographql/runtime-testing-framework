use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    info_status,
    kubernetes::Client,
};
use anyhow::Context;
use rep_orchestrator_shared::status::Status;

pub async fn create_service_account(namespace: &str, ctx: &impl CliContext) -> CliResult<()> {
    info_status!(
        ctx,
        Status::Provisioning,
        "creating results-writer service account in namespace '{namespace}'"
    )?;

    ctx.kube_client()
        .create_results_writer_service_account(namespace)
        .await
        .context("Failed to create results-writer service account.")
        .map_err(CliError::unrunnable)?;

    info_status!(
        ctx,
        Status::Provisioning,
        "results-writer service account created successfully"
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::mocks::MockContext,
        kubernetes::mocks::{KubeCall, MockClient as MockKubeClient},
        orchestrator::mocks::MockClient as MockOrchestrator,
    };
    use rep_orchestrator_shared::status::Status;

    #[tokio::test]
    async fn full_status_transition() {
        let ctx = MockContext::default();
        create_service_account("test-ns", &ctx).await.unwrap();

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[0].status, Status::Provisioning);
            assert_eq!(updates[1].status, Status::Provisioning);
        });
    }

    #[tokio::test]
    async fn calls_create_service_account_with_correct_namespace() {
        let ctx = MockContext::default();
        create_service_account("my-ns", &ctx).await.unwrap();

        ctx.kube_client.read_calls(|calls| {
            assert_eq!(
                calls,
                &[KubeCall::ApplyResultsWriterServiceAccount {
                    namespace: "my-ns".to_owned(),
                }]
            );
        });
    }

    #[tokio::test]
    async fn kube_failure_stops_after_initial_status() {
        let ctx = MockContext {
            kube_client: MockKubeClient::failing(),
            ..Default::default()
        };
        let err = create_service_account("test-ns", &ctx).await.unwrap_err();
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
        let err = create_service_account("test-ns", &ctx).await.unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }
}
