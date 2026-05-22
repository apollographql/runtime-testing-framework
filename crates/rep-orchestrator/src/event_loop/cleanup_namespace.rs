use crate::{
    db::TestExecution,
    event_loop::{Error, EventData, Result},
    k8s::{self, WorkloadClient},
};
use tracing::{info, warn};

pub(super) async fn try_run<K>(
    test_execution: TestExecution,
    clients: K,
) -> Result<Option<EventData>>
where
    K: WorkloadClient,
{
    let execution_id = test_execution.uuid();
    info!(%execution_id, "deleting namespace");
    match clients
        .delete_workload_namespace(&execution_id.to_string())
        .await
    {
        Ok(()) => Ok(None),
        Err(k8s::Error::Kube(kube::Error::Api(ref e))) if e.code == 404 => {
            warn!(%execution_id, "namespace not found, skipping delete");
            Ok(None)
        }
        Err(error) => Err(Error::DeleteNamespace { error }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::mock_client::{MockClient, Resp};

    #[tokio::test]
    async fn try_run_happy_path_returns_ok() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient::default_ok();

        let res = try_run(ex, clients).await;

        assert!(res.is_ok(), "{res:?}");
    }

    #[tokio::test]
    async fn try_run_returns_expected_delete_error() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            delete_workload_namespace: Resp::new(Err(k8s::Error::Kube(kube::Error::TlsRequired))),
            ..MockClient::default()
        };

        let res = try_run(ex, clients).await;

        assert!(matches!(res, Err(Error::DeleteNamespace { .. })));
    }

    #[tokio::test]
    async fn try_run_returns_ok_when_namespace_not_found() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            delete_workload_namespace: Resp::new(Err(k8s::Error::Kube(kube::Error::Api(
                Box::new(kube::core::Status {
                    status: None,
                    message: "namespaces not found".into(),
                    reason: "NotFound".into(),
                    code: 404,
                    details: None,
                    metadata: None,
                }),
            )))),
            ..MockClient::default()
        };

        let res = try_run(ex, clients).await;

        assert!(res.is_ok(), "{res:?}");
    }
}
