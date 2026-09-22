use crate::{
    db::TestExecution,
    event_loop::{ClusterId, Error, Event, EventData, Result},
    k8s::{self, WorkloadClient},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

pub(super) async fn try_run<K>(
    test_execution: TestExecution,
    clients: &mut K,
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

/// Wait for a deleted namespace's pods to actually be gone
pub(super) async fn wait_for_deletion<K>(
    test_execution: TestExecution,
    cluster: ClusterId,
    poll_interval_secs: u64,
    timeout_secs: u64,
    etx: UnboundedSender<Event>,
    clients: K,
) -> Result<Option<EventData>>
where
    K: WorkloadClient,
{
    let namespace = test_execution.uuid().to_string();

    tokio::spawn(async move {
        wait_and_notify(
            &namespace,
            test_execution,
            cluster,
            poll_interval_secs,
            timeout_secs,
            &etx,
            clients,
        )
        .await;
    });

    Ok(None)
}

async fn wait_and_notify<K>(
    namespace: &str,
    test_execution: TestExecution,
    cluster: ClusterId,
    poll_interval_secs: u64,
    timeout_secs: u64,
    etx: &UnboundedSender<Event>,
    mut clients: K,
) where
    K: WorkloadClient,
{
    let execution_id = test_execution.uuid();
    let deleted = clients
        .wait_for_namespace_pods_deleted(namespace, poll_interval_secs, timeout_secs)
        .await;

    if !deleted {
        warn!(
            %execution_id, timeout_secs,
            "timed out waiting for namespace pods to be deleted, freeing concurrency slot anyway"
        );
    }

    let _ = etx.send(Event {
        test_execution,
        cluster,
        data: EventData::MarkExecutionComplete,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::mock_client::{MockClient, Resp};
    use tokio::sync::mpsc;

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    #[tokio::test]
    async fn try_run_happy_path_returns_ok() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut clients = MockClient::default_ok();

        let res = try_run(ex, &mut clients).await;

        assert!(res.is_ok(), "{res:?}");
    }

    #[tokio::test]
    async fn try_run_returns_expected_delete_error() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut clients = MockClient {
            delete_workload_namespace: Resp::new(Err(k8s::Error::Kube(kube::Error::TlsRequired))),
            ..MockClient::default()
        };

        let res = try_run(ex, &mut clients).await;

        assert!(matches!(res, Err(Error::DeleteNamespace { .. })));
    }

    #[tokio::test]
    async fn try_run_returns_ok_when_namespace_not_found() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut clients = MockClient {
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

        let res = try_run(ex, &mut clients).await;

        assert!(res.is_ok(), "{res:?}");
    }

    #[tokio::test]
    async fn wait_and_notify_sends_mark_execution_complete_once_namespace_is_gone() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            wait_for_namespace_pods_deleted: Resp::new(true),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_notify("test-namespace", ex, alpha_cluster(), 10, 90, &etx, clients).await;

        let evt = erx.try_recv().unwrap();
        assert!(
            matches!(evt.data, EventData::MarkExecutionComplete),
            "{evt:?}"
        );
    }

    #[tokio::test]
    async fn wait_and_notify_sends_mark_execution_complete_even_on_timeout() {
        // Timing out must not leave the concurrency slot wedged forever - the caller still needs
        // to hear back so it can free the slot.
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            wait_for_namespace_pods_deleted: Resp::new(false),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_notify("test-namespace", ex, alpha_cluster(), 10, 90, &etx, clients).await;

        let evt = erx.try_recv().unwrap();
        assert!(
            matches!(evt.data, EventData::MarkExecutionComplete),
            "{evt:?}"
        );
    }
}
