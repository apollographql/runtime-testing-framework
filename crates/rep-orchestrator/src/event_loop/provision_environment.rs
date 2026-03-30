use crate::{
    db::{TestExecution, UpdateHandle},
    event_loop::{Error, Event, EventData, Result},
    k8s::{
        self, CLUSTER_API_NAMESPACE, Cluster, ENVIRONMENT_CONFIG_FILENAME, WatchOutcome,
        WorkflowSpec, env_configmap_name, workflow_name,
    },
};
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario, EnvironmentConfig};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

const MSG_CREATE_ENV_CM: &str = "creating environment configmap";
const MSG_ARGO_CREATE: &str = "creating Argo workflow";
const MSG_ARGO_WAIT: &str = "waiting for Argo workflow to complete";

pub(super) async fn try_run<K, H>(
    test_execution: TestExecution,
    environment: DockerComposeEnvironment,
    scenario: DockerScenario,
    etx: UnboundedSender<Event>,
    clients: K,
    conn: &mut H,
) -> Result<()>
where
    K: k8s::Client,
    H: UpdateHandle,
{
    let execution_id = test_execution.uuid();
    let content = serde_yaml::to_string(&EnvironmentConfig {
        name: execution_id.to_string(),
        execution: environment,
        description: Default::default(),
        variable_definitions: Default::default(),
        custom_providers: Default::default(),
    })
    .unwrap_or_else(|e| panic!("EnvironmentConfig failed to serialize: {e}"));

    info!(%execution_id, "creating environment configmap");
    conn.mark_execution_as_provisioning(&test_execution, MSG_CREATE_ENV_CM.to_string())
        .await;

    clients
        .create_configmap(
            Cluster::Management,
            CLUSTER_API_NAMESPACE,
            &env_configmap_name(&execution_id),
            ENVIRONMENT_CONFIG_FILENAME,
            content,
        )
        .await
        .map_err(|error| Error::CreateConfigmap {
            kind: "environment",
            error,
        })?;

    info!(%execution_id, "creating environment argo workflow");
    conn.mark_execution_as_provisioning(&test_execution, MSG_ARGO_CREATE.to_string())
        .await;

    clients
        .create_argo_workflow(
            &workflow_name(&execution_id),
            WorkflowSpec::for_execution_id(&execution_id),
        )
        .await
        .map_err(|error| Error::CreateArgoWorkflow { error })?;

    conn.mark_execution_as_provisioning(&test_execution, MSG_ARGO_WAIT.to_string())
        .await;

    info!(%execution_id, "waiting for environment argo workflow to complete");
    tokio::spawn(async move {
        wait_and_update(test_execution, scenario, &etx, clients).await;
    });

    Ok(())
}

async fn wait_and_update<K>(
    test_execution: TestExecution,
    scenario: DockerScenario,
    etx: &UnboundedSender<Event>,
    clients: K,
) where
    K: k8s::Client,
{
    let execution_id = test_execution.uuid();
    let data = match clients.wait_for_workflow(&test_execution.uuid()).await {
        WatchOutcome::Succeeded => {
            info!(%execution_id, "argo workflow completed successfully");
            EventData::RunScenario(scenario)
        }

        WatchOutcome::Failed(reason) => {
            warn!(%execution_id, %reason, "argo workflow failed");
            EventData::MarkUnrunnable(WatchOutcome::Failed(reason).to_string())
        }

        outcome => {
            warn!(%execution_id, %outcome, "unable to determine state of argo workflow");
            EventData::MarkUnrunnable(outcome.to_string())
        }
    };

    let _ = etx.send(Event {
        test_execution,
        data,
    });
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
    use rtf_config::{
        formats::{DockerCommand, DockerScenario},
        templating::Field,
    };
    use simple_test_case::test_case;
    use tokio::sync::mpsc;

    fn stub_environment() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
            file_providers: vec![],
            env_vars: Default::default(),
        }
    }

    fn stub_scenario() -> DockerScenario {
        DockerScenario {
            docker: DockerCommand {
                image: Field::Resolved("nginx".into()),
                tag: None,
                command: Field::Resolved("echo test".into()),
            },
            env_vars: Default::default(),
            file_providers: vec![],
        }
    }

    #[tokio::test]
    async fn try_run_happy_path_sets_expected_statuses() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::default_ok();
        let (etx, _erx) = mpsc::unbounded_channel();

        let res = try_run(
            ex,
            stub_environment(),
            stub_scenario(),
            etx,
            clients,
            &mut handle,
        )
        .await;

        assert!(res.is_ok(), "{res:?}");
        assert_eq!(
            &handle.status_updates,
            &[
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_CREATE_ENV_CM)),
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_ARGO_CREATE)),
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_ARGO_WAIT)),
            ]
        );
    }

    #[tokio::test]
    async fn try_run_returns_expected_configmap_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient {
            create_configmap: Resp::new(Err(k8s::Error::Kube(kube::Error::TlsRequired))),
            ..MockClient::default()
        };
        let (etx, _erx) = mpsc::unbounded_channel();

        let res = try_run(
            ex,
            stub_environment(),
            stub_scenario(),
            etx,
            clients.clone(),
            &mut handle,
        )
        .await;

        assert!(matches!(
            res,
            Err(Error::CreateConfigmap {
                kind: "environment",
                ..
            })
        ));
        assert_eq!(
            &handle.status_updates,
            &[TaggedStatusUpdate::execution(
                1,
                Status::Provisioning,
                Some(MSG_CREATE_ENV_CM)
            )]
        );
    }

    #[tokio::test]
    async fn try_run_returns_expected_workflow_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient {
            create_workflow: Resp::new(Err(k8s::Error::Kube(kube::Error::TlsRequired))),
            ..MockClient::default_ok()
        };
        let (etx, _erx) = mpsc::unbounded_channel();

        let res = try_run(
            ex,
            stub_environment(),
            stub_scenario(),
            etx,
            clients,
            &mut handle,
        )
        .await;

        assert!(matches!(res, Err(Error::CreateArgoWorkflow { .. })));
        assert_eq!(
            &handle.status_updates,
            &[
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_CREATE_ENV_CM)),
                TaggedStatusUpdate::execution(1, Status::Provisioning, Some(MSG_ARGO_CREATE)),
            ]
        );
    }

    #[tokio::test]
    async fn wait_and_update_submits_run_scenario_on_success() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let clients = MockClient {
            wait_for_workflow: Resp::new(WatchOutcome::Succeeded),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_update(ex, stub_scenario(), &etx, clients).await;

        let evt = erx.try_recv().unwrap();
        assert!(matches!(evt.data, EventData::RunScenario(_)), "{evt:?}");
    }

    #[test_case(WatchOutcome::Failed(String::new()); "failed")]
    #[test_case(WatchOutcome::WatcherError(String::new()); "watch error")]
    #[test_case(WatchOutcome::StreamClosed; "stream closed")]
    #[tokio::test]
    async fn wait_and_update_submits_mark_unrunnable_on_watch_error(outcome: WatchOutcome) {
        let ex = TestExecution::create_stub(1, 1, "test");
        let clients = MockClient {
            wait_for_workflow: Resp::new(outcome),
            ..MockClient::default_ok()
        };
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_update(ex, stub_scenario(), &etx, clients).await;

        let evt = erx.try_recv().unwrap();
        assert!(matches!(evt.data, EventData::MarkUnrunnable(_)), "{evt:?}");
    }
}
