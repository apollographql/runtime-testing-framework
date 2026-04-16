use crate::{
    context::CliContext,
    error::{CliError, CliResult},
    info_status,
    kubernetes::Client,
};
use anyhow::Context;
use rep_orchestrator_shared::status::Status;

pub async fn create_namespace(namespace: &str, ctx: &impl CliContext) -> CliResult<()> {
    info_status!(
        ctx,
        Status::Provisioning,
        "Creating namespace '{namespace}' in workload cluster..."
    )?;

    ctx.kube_client()
        .create_namespace(namespace)
        .await
        .context("Failed to create kube namespace.")
        .map_err(CliError::unrunnable)?;

    info_status!(ctx, Status::Provisioning, "Namespace created successfully.")?;

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
        create_namespace("test-ns", &ctx).await.unwrap();

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[0].status, Status::Provisioning);
            assert_eq!(updates[1].status, Status::Provisioning);
        });
    }

    #[tokio::test]
    async fn calls_apply_namespace_with_correct_name() {
        let ctx = MockContext::default();
        create_namespace("my-ns", &ctx).await.unwrap();

        ctx.kube_client.read_calls(|calls| {
            assert_eq!(
                calls,
                &[KubeCall::ApplyNamespace {
                    name: "my-ns".to_owned(),
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
        let err = create_namespace("test-ns", &ctx).await.unwrap_err();
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
        let err = create_namespace("test-ns", &ctx).await.unwrap_err();
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }
}
