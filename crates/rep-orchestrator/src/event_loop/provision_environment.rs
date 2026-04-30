use crate::{
    db::{TestExecution, UpdateHandle},
    event_loop::{Error, Event, EventData, Result},
    k8s::{self, WatchOutcome, WorkflowSpec},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

pub(crate) const MSG_CREATE_ARGO: &str = "creating Argo workflow";
pub(crate) const MSG_ARGO_CREATED: &str = "Argo workflow created";
pub const MSG_ARGO_WAIT: &str = "waiting for Argo workflow to complete";
pub(crate) const MSG_ARGO_COMPLETE: &str = "Argo workflow complete";

pub(super) async fn create_workflow<K, H>(
    test_execution: TestExecution,
    orchestrator_url: &str,
    kubeconfig_secret_name: &str,
    clients: K,
    conn: &mut H,
) -> Result<Option<EventData>>
where
    K: k8s::Client,
    H: UpdateHandle,
{
    let execution_id = test_execution.uuid();

    info!(%execution_id, "creating environment argo workflow");
    conn.mark_execution_as_provisioning(&test_execution, MSG_CREATE_ARGO.to_string())
        .await;

    clients
        .create_argo_workflow(
            &execution_id,
            WorkflowSpec::for_execution(&test_execution, orchestrator_url, kubeconfig_secret_name),
        )
        .await
        .map_err(|error| Error::CreateArgoWorkflow { error })?;

    conn.mark_execution_as_provisioning(&test_execution, MSG_ARGO_CREATED.to_string())
        .await;

    Ok(Some(EventData::WaitForEnvArgoWorkflow))
}

pub(super) async fn wait_for_workflow<K, H>(
    test_execution: TestExecution,
    etx: UnboundedSender<Event>,
    clients: K,
    conn: &mut H,
) -> Result<Option<EventData>>
where
    K: k8s::Client,
    H: UpdateHandle,
{
    let execution_id = test_execution.uuid();

    info!(%execution_id, "waiting for environment argo workflow to complete");
    conn.mark_execution_as_provisioning(&test_execution, MSG_ARGO_WAIT.to_string())
        .await;

    tokio::spawn(async move {
        wait_and_update(test_execution, &etx, clients).await;
    });

    Ok(None)
}

async fn wait_and_update<K>(test_execution: TestExecution, etx: &UnboundedSender<Event>, clients: K)
where
    K: k8s::Client,
{
    let execution_id = test_execution.uuid();
    let to_send = match clients.wait_for_workflow(&test_execution.uuid()).await {
        WatchOutcome::Succeeded => {
            info!(%execution_id, "argo workflow completed successfully");
            vec![
                EventData::ArgoWorkflowComplete,
                EventData::CreateScenarioJob,
            ]
        }

        WatchOutcome::Failed(reason) => {
            warn!(%execution_id, %reason, "argo workflow failed");
            vec![
                EventData::MarkUnrunnable(WatchOutcome::Failed(reason).to_string()),
                EventData::CleanupNamespace,
            ]
        }

        WatchOutcome::ContainerUnrunnable(reason) => {
            warn!(%execution_id, %reason, "container unrunnable");
            vec![
                EventData::MarkUnrunnable(WatchOutcome::ContainerUnrunnable(reason).to_string()),
                EventData::CleanupNamespace,
            ]
        }

        outcome => {
            warn!(%execution_id, %outcome, "unable to determine state of argo workflow");
            vec![
                EventData::MarkUnrunnable(outcome.to_string()),
                EventData::CleanupNamespace,
            ]
        }
    };

    for data in to_send.into_iter() {
        let _ = etx.send(Event {
            test_execution: test_execution.clone(),
            data,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{MockUpdateHandle, Status, TaggedStatusUpdate},
        k8s::{
            self,
            mock_client::{MockClient, Resp},
        },
    };
    use simple_test_case::test_case;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn full_happy_path_sets_expected_statuses() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::default_ok();
        let (etx, _erx) = mpsc::unbounded_channel();

        // create workflow
        let res = create_workflow(
            ex.clone(),
            "http://localhost:8035",
            "workload-kubeconfig",
            clients.clone(),
            &mut handle,
        )
        .await;
        assert!(res.is_ok(), "create_workflow: {res:?}");

        // wait for workflow to complete
        let res = wait_for_workflow(ex, etx, clients, &mut handle).await;
        assert!(res.is_ok(), "wait for workflow: {res:?}");

        assert_eq!(
            &handle.status_updates,
            &[
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_CREATE_ARGO)),
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_ARGO_CREATED)),
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_ARGO_WAIT)),
            ]
        );
    }

    #[tokio::test]
    async fn create_workflow_expected_workflow_error() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient {
            create_workflow: Resp::new(Err(k8s::Error::Kube(kube::Error::TlsRequired))),
            ..MockClient::default_ok()
        };

        let res = create_workflow(
            ex,
            "http://localhost:8035",
            "workload-kubeconfig",
            clients,
            &mut handle,
        )
        .await;

        assert!(matches!(res, Err(Error::CreateArgoWorkflow { .. })));
        assert_eq!(
            &handle.status_updates,
            &[TaggedStatusUpdate::execution(
                1,
                Status::Provisioning,
                Some(MSG_CREATE_ARGO)
            ),]
        );
    }

    #[tokio::test]
    async fn wait_and_update_submits_expected_events_on_success() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            wait_for_workflow: Resp::new(WatchOutcome::Succeeded),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_update(ex, &etx, clients).await;

        // should get two events: workflow complete and create scenario configmap
        let evt = erx.try_recv().unwrap();
        assert!(
            matches!(evt.data, EventData::ArgoWorkflowComplete),
            "{evt:?}"
        );

        let evt = erx.try_recv().unwrap();
        assert!(matches!(evt.data, EventData::CreateScenarioJob), "{evt:?}");
    }

    #[test_case(WatchOutcome::Failed(String::new()); "failed")]
    #[test_case(WatchOutcome::ContainerUnrunnable("ImagePullBackOff".into()); "container unrunnable")]
    #[test_case(WatchOutcome::WatcherError(String::new()); "watch error")]
    #[test_case(WatchOutcome::StreamClosed; "stream closed")]
    #[tokio::test]
    async fn wait_and_update_submits_mark_unrunnable_then_cleanup_on_watch_error(
        outcome: WatchOutcome,
    ) {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            wait_for_workflow: Resp::new(outcome),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_update(ex, &etx, clients).await;

        let first = erx.try_recv().unwrap();
        let second = erx.try_recv().unwrap();
        assert!(
            matches!(first.data, EventData::MarkUnrunnable(_)),
            "first event: {first:?}"
        );
        assert!(
            matches!(second.data, EventData::CleanupNamespace),
            "second event: {second:?}"
        );
    }
}
