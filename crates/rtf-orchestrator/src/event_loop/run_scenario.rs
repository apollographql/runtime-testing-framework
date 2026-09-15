use crate::{
    config::ClusterRoles,
    db::{TestExecution, UpdateHandle},
    event_loop::{ClusterId, Error, Event, EventData, Result},
    k8s::{WatchOutcome, WorkloadClient, scenario_job},
};
use rtf_orchestrator_shared::SCENARIO_JOB_NAME;
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

pub(crate) const MSG_CREATE_JOB: &str = "creating scenario job";
pub(crate) const MSG_JOB_CREATED: &str = "scenario job created";
pub const MSG_JOB_WAIT: &str = "waiting for scenario job to complete";

pub(crate) struct CreateJobConfig<'a> {
    pub(crate) orchestrator_url: &'a str,
    pub(crate) prometheus_endpoint: &'a str,
    pub(crate) toolbox_pull_policy: &'a str,
    pub(crate) toolbox_image: &'a str,
    pub(crate) cluster_roles: &'a ClusterRoles,
    pub(crate) allow_namespace_write: bool,
    pub(crate) exclusive_nodes: bool,
    pub(crate) scenario_node_selector: &'a BTreeMap<String, String>,
}

pub(super) async fn create_job<K, H>(
    test_execution: TestExecution,
    scenario_image: String,
    scenario_command: String,
    config: &CreateJobConfig<'_>,
    clients: &mut K,
    conn: &mut H,
) -> Result<Option<EventData>>
where
    K: WorkloadClient,
    H: UpdateHandle,
{
    let execution_id = test_execution.uuid();
    let namespace = execution_id.to_string();

    info!(%execution_id, "creating scenario job");
    conn.mark_execution_as_environment_ready(&test_execution, MSG_CREATE_JOB.to_string())
        .await;

    match clients
        .create_job(
            &namespace,
            SCENARIO_JOB_NAME,
            &execution_id,
            config.allow_namespace_write,
            config.cluster_roles,
            scenario_job(&test_execution, scenario_image, scenario_command, config),
        )
        .await
    {
        Ok(_) => {}

        // It is possible for the job to already exist if it was previously created before a server
        // restart / crash. As we name jobs deterministically based on the execution ID, we know
        // that the pre-existing job is the one we need so we move directly to waiting for it to
        // complete.
        Err(e) if e.is_409_conflict() => return Ok(Some(EventData::WaitForScenarioJob)),

        Err(error) => return Err(Error::CreateJob { error }),
    }

    info!(%execution_id, "scenario job created");
    conn.mark_execution_as_environment_ready(&test_execution, MSG_JOB_CREATED.to_string())
        .await;

    Ok(Some(EventData::WaitForScenarioJob))
}

#[expect(clippy::too_many_arguments)]
pub(super) async fn wait_for_job<K, H>(
    test_execution: TestExecution,
    cluster: ClusterId,
    failed_execution_ttl_seconds: u64,
    poll_interval_secs: u64,
    retry_window_secs: u64,
    etx: UnboundedSender<Event>,
    clients: K,
    conn: &mut H,
) -> Result<Option<EventData>>
where
    K: WorkloadClient,
    H: UpdateHandle,
{
    let execution_id = test_execution.uuid();
    let namespace = execution_id.to_string();

    info!(%execution_id, "waiting for scenario job to complete");
    conn.mark_execution_as_environment_ready(&test_execution, MSG_JOB_WAIT.to_string())
        .await;

    tokio::spawn(async move {
        wait_and_update(
            &namespace,
            test_execution,
            cluster,
            failed_execution_ttl_seconds,
            poll_interval_secs,
            retry_window_secs,
            &etx,
            clients,
        )
        .await;
    });

    Ok(None)
}

#[expect(clippy::too_many_arguments)]
async fn wait_and_update<K>(
    namespace: &str,
    test_execution: TestExecution,
    cluster: ClusterId,
    failed_execution_ttl_seconds: u64,
    poll_interval_secs: u64,
    retry_window_secs: u64,
    etx: &UnboundedSender<Event>,
    mut clients: K,
) where
    K: WorkloadClient,
{
    let execution_id = test_execution.uuid();

    let to_send = match clients
        .wait_for_job(
            namespace,
            &test_execution.uuid(),
            poll_interval_secs,
            retry_window_secs,
        )
        .await
    {
        WatchOutcome::Succeeded => {
            info!(%execution_id, "job completed successfully");
            vec![EventData::CleanupNamespace]
        }

        WatchOutcome::Failed(reason) => {
            warn!(%execution_id, %reason, "job failed");
            vec![
                EventData::MarkUnrunnable(WatchOutcome::Failed(reason).to_string()),
                EventData::CleanupNamespaceAfter(failed_execution_ttl_seconds),
            ]
        }

        WatchOutcome::ContainerUnrunnable(reason) => {
            warn!(%execution_id, %reason, "container unrunnable");

            vec![
                EventData::MarkUnrunnable(reason.to_string()),
                EventData::CleanupNamespaceAfter(failed_execution_ttl_seconds),
            ]
        }

        outcome => {
            warn!(%execution_id, %outcome, "unable to determine state of job");
            vec![
                EventData::MarkUnrunnable(outcome.to_string()),
                EventData::CleanupNamespaceAfter(failed_execution_ttl_seconds),
            ]
        }
    };

    for data in to_send.into_iter() {
        let _ = etx.send(Event {
            test_execution: test_execution.clone(),
            cluster: cluster.clone(),
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

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    #[tokio::test]
    async fn full_happy_path_sets_expected_statuses() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let mut clients = MockClient::default_ok();
        let (etx, _erx) = mpsc::unbounded_channel();

        // create job
        let res = create_job(
            ex.clone(),
            "image".into(),
            "cmd".into(),
            &CreateJobConfig {
                orchestrator_url: "http://localhost:8035",
                prometheus_endpoint: "http://prometheus:9090",
                toolbox_pull_policy: "IfNotPresent",
                toolbox_image: "rtf-toolbox:edge",
                cluster_roles: &ClusterRoles {
                    cluster_read: "scenario-cluster-read".into(),
                    namespace_read: "scenario-namespace-read".into(),
                    namespace_write: "scenario-namespace-write".into(),
                },
                allow_namespace_write: false,
                exclusive_nodes: false,
                scenario_node_selector: &BTreeMap::new(),
            },
            &mut clients,
            &mut handle,
        )
        .await;
        assert!(res.is_ok(), "create_job: {res:?}");

        // wait for job to complete
        let res = wait_for_job(ex, alpha_cluster(), 600, 10, 300, etx, clients, &mut handle).await;
        assert!(res.is_ok(), "wait_for_job: {res:?}");

        use Status::*;

        assert_eq!(
            &handle.status_updates,
            &[
                TaggedStatusUpdate::execution(1, EnvironmentReady, Some(MSG_CREATE_JOB)),
                TaggedStatusUpdate::execution(1, EnvironmentReady, Some(MSG_JOB_CREATED)),
                TaggedStatusUpdate::execution(1, EnvironmentReady, Some(MSG_JOB_WAIT)),
            ]
        );
    }

    #[tokio::test]
    async fn create_job_returns_expected_job_error() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let mut clients = MockClient {
            create_job: Resp::new(Err(k8s::Error::Kube(kube::Error::TlsRequired))),
            ..MockClient::default_ok()
        };

        let res = create_job(
            ex.clone(),
            "image".into(),
            "cmd".into(),
            &CreateJobConfig {
                orchestrator_url: "http://localhost:8035",
                prometheus_endpoint: "http://prometheus:9090",
                toolbox_pull_policy: "IfNotPresent",
                toolbox_image: "rtf-toolbox:edge",
                cluster_roles: &ClusterRoles {
                    cluster_read: "scenario-cluster-read".into(),
                    namespace_read: "scenario-namespace-read".into(),
                    namespace_write: "scenario-namespace-write".into(),
                },
                allow_namespace_write: false,
                exclusive_nodes: false,
                scenario_node_selector: &BTreeMap::new(),
            },
            &mut clients,
            &mut handle,
        )
        .await;

        assert!(matches!(res, Err(Error::CreateJob { .. })));
        assert_eq!(
            &handle.status_updates,
            &[TaggedStatusUpdate::execution(
                1,
                Status::EnvironmentReady,
                Some(MSG_CREATE_JOB)
            ),]
        );
    }

    #[tokio::test]
    async fn wait_and_update_submits_cleanup_namespace_on_success() {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            wait_for_job: Resp::new(WatchOutcome::Succeeded),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_update(
            "test-namespace",
            ex,
            alpha_cluster(),
            600,
            10,
            300,
            &etx,
            clients,
        )
        .await;

        let evt = erx.try_recv().unwrap();
        assert!(matches!(evt.data, EventData::CleanupNamespace), "{evt:?}");
    }

    #[test_case(WatchOutcome::Failed(String::new()); "failed")]
    #[test_case(WatchOutcome::WatchErrors(String::new()); "transient error limit exceeded")]
    #[test_case(WatchOutcome::ContainerUnrunnable("ImagePullBackOff".into()); "container unrunnable")]
    #[tokio::test]
    async fn wait_and_update_submits_mark_unrunnable_then_cleanup_on_watch_error(
        outcome: WatchOutcome,
    ) {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let clients = MockClient {
            wait_for_job: Resp::new(outcome),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_update(
            "test-namespace",
            ex,
            alpha_cluster(),
            600,
            10,
            300,
            &etx,
            clients,
        )
        .await;

        let first = erx.try_recv().unwrap();
        let second = erx.try_recv().unwrap();
        assert!(
            matches!(first.data, EventData::MarkUnrunnable(_)),
            "first event: {first:?}"
        );
        assert!(
            matches!(second.data, EventData::CleanupNamespaceAfter(600)),
            "second event: {second:?}"
        );
    }
}
