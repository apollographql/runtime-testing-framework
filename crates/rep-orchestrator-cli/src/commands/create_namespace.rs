use crate::{context::CliContext, info_status, kubernetes::Client};
use rep_orchestrator_shared::status::Status;

pub async fn create_namespace(namespace: &str, ctx: &impl CliContext) -> crate::Result<()> {
    info_status!(
        ctx,
        Status::Provisioning,
        "creating namespace '{namespace}' in workload cluster"
    )?;

    ctx.kube_client().create_namespace(namespace).await?;
    info_status!(ctx, Status::Provisioning, "namespace created successfully")?;

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
        create_namespace("test-ns", &ctx).await.unwrap();

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[0], Status::Provisioning);
            assert_eq!(updates[1], Status::Provisioning);
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
        let res = create_namespace("test-ns", &ctx).await;
        assert!(res.is_err(), "{res:?}");

        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0], Status::Provisioning);
        });
    }

    #[tokio::test]
    async fn aborts_on_status_update_failure() {
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::failing(),
            ..Default::default()
        };
        let res = create_namespace("test-ns", &ctx).await;
        assert!(res.is_err(), "{res:?}");
    }
}
