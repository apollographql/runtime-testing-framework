use crate::{
    db::TestExecution,
    event_loop::{Error, EventData, Result},
    k8s,
};
use tracing::info;

pub(super) async fn try_run<K>(
    test_execution: TestExecution,
    clients: K,
) -> Result<Option<EventData>>
where
    K: k8s::Client,
{
    let execution_id = test_execution.uuid();
    info!(%execution_id, "deleting namespace");
    clients
        .delete_workload_namespace(&execution_id.to_string())
        .await
        .map_err(|error| Error::DeleteNamespace { error })?;

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::mock_client::{MockClient, Resp};

    #[tokio::test]
    async fn try_run_happy_path_returns_ok() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let clients = MockClient::default_ok();

        let res = try_run(ex, clients).await;

        assert!(res.is_ok(), "{res:?}");
    }

    #[tokio::test]
    async fn try_run_returns_expected_delete_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let clients = MockClient {
            delete_workload_namespace: Resp::new(Err(k8s::Error::Kube(kube::Error::TlsRequired))),
            ..MockClient::default()
        };

        let res = try_run(ex, clients).await;

        assert!(matches!(res, Err(Error::DeleteNamespace { .. })));
    }
}
