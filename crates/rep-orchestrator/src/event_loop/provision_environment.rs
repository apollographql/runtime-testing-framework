use crate::{
    conn,
    db::{Status, TestExecution, UpdateHandle},
    event_loop::{Event, EventType},
    k8s::{
        self, CLUSTER_API_NAMESPACE, Cluster, Dag, ENVIRONMENT_CONFIG_FILENAME, MainTemplate,
        TOOLBOX_IMAGE, TaskSpec, TaskTemplate, TemplateDef, WatchOutcome, WorkflowSpec,
        env_configmap_name, workflow_name,
    },
};
use k8s_openapi::api::core::v1::{
    ConfigMapVolumeSource, Container, KeyToPath, SecretVolumeSource, Volume, VolumeMount,
};
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario, EnvironmentConfig};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, warn};
use uuid::Uuid;

pub async fn run<K>(
    test_execution: TestExecution,
    environment: DockerComposeEnvironment,
    scenario: DockerScenario,
    etx: UnboundedSender<Event>,
    clients: &K,
) where
    K: k8s::Client + Clone + Send + 'static,
{
    let execution_id = test_execution.uuid();
    let result: crate::Result<()> = async {
        try_run(test_execution, environment, scenario, etx, clients, conn!()).await;
        Ok(())
    }
    .await;

    if let Err(e) = result {
        error!(%e, %execution_id, "failed to acquire DB connection for ProvisionEnvironment");
    }
}

async fn try_run<K, H>(
    test_execution: TestExecution,
    environment: DockerComposeEnvironment,
    scenario: DockerScenario,
    etx: UnboundedSender<Event>,
    clients: &K,
    db: &mut H,
) where
    K: k8s::Client + Clone + Send + 'static,
    H: UpdateHandle,
{
    let execution_id = test_execution.uuid();

    let env_config = EnvironmentConfig {
        name: execution_id.to_string(),
        description: String::new(),
        variable_definitions: vec![],
        custom_providers: vec![],
        execution: environment,
    };
    let content = match serde_yaml::to_string(&env_config) {
        Ok(c) => c,
        Err(e) => {
            let _ = db
                .update_test_execution_status(
                    &test_execution,
                    Status::Unrunnable,
                    Some(e.to_string()),
                )
                .await;
            return;
        }
    };

    if let Err(e) = clients
        .create_configmap(
            Cluster::Management,
            CLUSTER_API_NAMESPACE,
            &env_configmap_name(&execution_id),
            ENVIRONMENT_CONFIG_FILENAME,
            content,
        )
        .await
    {
        let _ = db
            .update_test_execution_status(&test_execution, Status::Unrunnable, Some(e.to_string()))
            .await;
        return;
    }

    if let Err(e) = db
        .update_test_execution_status(&test_execution, Status::Provisioning, None)
        .await
    {
        error!(%e, %execution_id, "failed to set execution to Provisioning");
        return;
    }

    if let Err(e) = clients
        .create_argo_workflow(
            &workflow_name(&execution_id),
            build_workflow_spec(&execution_id),
        )
        .await
    {
        let _ = db
            .update_test_execution_status(&test_execution, Status::Unrunnable, Some(e.to_string()))
            .await;
        return;
    }

    let ex = test_execution.clone();
    let clients_spawn = clients.clone();
    tokio::spawn(async move {
        let result: crate::Result<()> = async {
            wait_and_update(&ex, scenario, &etx, &clients_spawn, conn!()).await;
            Ok(())
        }
        .await;
        if let Err(e) = result {
            error!(%e, execution_id = %ex.uuid(), "failed to acquire DB connection for wait_and_update");
        }
    });
}

async fn wait_and_update<K, H>(
    test_execution: &TestExecution,
    scenario: DockerScenario,
    etx: &UnboundedSender<Event>,
    clients: &K,
    db: &mut H,
) where
    K: k8s::Client,
    H: UpdateHandle,
{
    let execution_id = test_execution.uuid();

    let outcome = clients.wait_for_workflow(&execution_id).await;

    if let Err(e) = clients
        .delete_management_configmap(CLUSTER_API_NAMESPACE, &env_configmap_name(&execution_id))
        .await
    {
        warn!(%e, %execution_id, "failed to delete environment configmap after workflow completion");
    }

    match outcome {
        WatchOutcome::Succeeded => {
            let _ = db
                .update_test_execution_status(test_execution, Status::Running, None)
                .await;
            let _ = etx.send(Event {
                test_execution: test_execution.clone(),
                ty: EventType::RunScenario(scenario),
            });
        }
        WatchOutcome::Failed(msg) => {
            let _ = db
                .update_test_execution_status(test_execution, Status::Unrunnable, Some(msg))
                .await;
        }
        WatchOutcome::WatcherError(msg) => {
            let _ = db
                .update_test_execution_status(test_execution, Status::Unrunnable, Some(msg))
                .await;
        }
        WatchOutcome::StreamClosed => {
            let _ = db
                .update_test_execution_status(
                    test_execution,
                    Status::Unrunnable,
                    Some("workflow watcher stream closed unexpectedly".into()),
                )
                .await;
        }
    }
}

fn build_workflow_spec(execution_id: &Uuid) -> WorkflowSpec {
    let ns = execution_id.to_string();

    let kubeconfig_mount = VolumeMount {
        name: "kubeconfig".into(),
        mount_path: "/kubeconfig".into(),
        ..Default::default()
    };
    let gcr_secret_mount = VolumeMount {
        name: "gcr-secret".into(),
        mount_path: "/gcr-secret".into(),
        ..Default::default()
    };
    let environment_mount = VolumeMount {
        name: "environment".into(),
        mount_path: "/environment".into(),
        ..Default::default()
    };

    let create_namespace = TaskTemplate {
        name: "create-namespace".into(),
        container: Container {
            image: Some(TOOLBOX_IMAGE.to_owned()),
            command: Some(vec!["rep-orchestrator-cli".into()]),
            args: Some(vec![
                "create-namespace".into(),
                "--namespace".into(),
                ns.clone(),
                "--kubeconfig".into(),
                "/kubeconfig/value".into(),
            ]),
            volume_mounts: Some(vec![kubeconfig_mount.clone()]),
            ..Default::default()
        },
        volumes: None,
    };

    let create_pull_secret = TaskTemplate {
        name: "create-pull-secret".into(),
        container: Container {
            image: Some(TOOLBOX_IMAGE.to_owned()),
            command: Some(vec!["rep-orchestrator-cli".into()]),
            args: Some(vec![
                "create-pull-secret".into(),
                "--namespace".into(),
                ns.clone(),
                "--kubeconfig".into(),
                "/kubeconfig/value".into(),
                "--docker-config".into(),
                "/gcr-secret/config.json".into(),
            ]),
            volume_mounts: Some(vec![kubeconfig_mount.clone(), gcr_secret_mount.clone()]),
            ..Default::default()
        },
        volumes: None,
    };

    let deploy_environment = TaskTemplate {
        name: "deploy-environment".into(),
        container: Container {
            image: Some(TOOLBOX_IMAGE.to_owned()),
            command: Some(vec!["rep-orchestrator-cli".into()]),
            args: Some(vec![
                "deploy-environment".into(),
                "--namespace".into(),
                ns.clone(),
                "--kubeconfig".into(),
                "/kubeconfig/value".into(),
                "--environment".into(),
                "/environment/environment.yaml".into(),
                "--timeout".into(),
                "300".into(),
            ]),
            volume_mounts: Some(vec![kubeconfig_mount, environment_mount]),
            ..Default::default()
        },
        volumes: None,
    };

    WorkflowSpec {
        service_account_name: "argo-workflow".into(),
        entrypoint: "main".into(),
        on_exit: "".into(),
        templates: vec![
            TemplateDef::Main(MainTemplate {
                name: "main".into(),
                dag: Dag {
                    tasks: vec![
                        TaskSpec {
                            name: "create-namespace".into(),
                            template: "create-namespace".into(),
                            dependencies: vec![],
                        },
                        TaskSpec {
                            name: "create-pull-secret".into(),
                            template: "create-pull-secret".into(),
                            dependencies: vec!["create-namespace".into()],
                        },
                        TaskSpec {
                            name: "deploy-environment".into(),
                            template: "deploy-environment".into(),
                            dependencies: vec!["create-pull-secret".into()],
                        },
                    ],
                },
            }),
            TemplateDef::Task(create_namespace),
            TemplateDef::Task(create_pull_secret),
            TemplateDef::Task(deploy_environment),
        ],
        volumes: vec![
            Volume {
                name: "kubeconfig".into(),
                secret: Some(SecretVolumeSource {
                    secret_name: Some("workload-kubeconfig".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Volume {
                name: "gcr-secret".into(),
                secret: Some(SecretVolumeSource {
                    secret_name: Some("gcr-secret".into()),
                    items: Some(vec![KeyToPath {
                        key: ".dockerconfigjson".into(),
                        path: "config.json".into(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Volume {
                name: "environment".into(),
                config_map: Some(ConfigMapVolumeSource {
                    name: env_configmap_name(execution_id),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{MockUpdateHandle, TaggedStatusUpdate},
        k8s::{WatchOutcome, mock_client::MockClient},
    };
    use rtf_config::{
        formats::{DockerCommand, DockerScenario},
        templating::Field,
    };
    use tokio::sync::mpsc;

    fn dummy_environment() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
            file_providers: vec![],
            env_vars: Default::default(),
        }
    }

    fn dummy_scenario() -> DockerScenario {
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

    fn assert_single_execution_status(updates: &[TaggedStatusUpdate], expected: Status) {
        assert_eq!(
            updates.len(),
            1,
            "expected exactly one status update, got: {updates:?}"
        );
        let TaggedStatusUpdate::Execution(_, ref s) = updates[0] else {
            panic!("expected Execution update, got: {:?}", updates[0]);
        };
        assert_eq!(s.status, expected);
    }

    fn execution_statuses(updates: &[TaggedStatusUpdate]) -> Vec<Status> {
        updates
            .iter()
            .map(|u| {
                let TaggedStatusUpdate::Execution(_, s) = u else {
                    panic!("expected Execution update, got: {u:?}");
                };
                s.status
            })
            .collect()
    }

    #[tokio::test]
    async fn try_run_sets_provisioning_on_success() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::ok();
        let (etx, _erx) = mpsc::unbounded_channel();

        try_run(
            ex,
            dummy_environment(),
            dummy_scenario(),
            etx,
            &clients,
            &mut handle,
        )
        .await;

        assert_single_execution_status(&handle.status_updates, Status::Provisioning);
    }

    #[tokio::test]
    async fn try_run_sets_failed_on_configmap_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::configmap_err();
        let (etx, _erx) = mpsc::unbounded_channel();

        try_run(
            ex,
            dummy_environment(),
            dummy_scenario(),
            etx,
            &clients,
            &mut handle,
        )
        .await;

        assert_single_execution_status(&handle.status_updates, Status::Unrunnable);
        assert!(
            clients.create_configmap_result.lock().unwrap().is_none(),
            "create_configmap must have been called"
        );
    }

    #[tokio::test]
    async fn try_run_sets_failed_on_workflow_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::workflow_err();
        let (etx, _erx) = mpsc::unbounded_channel();

        try_run(
            ex,
            dummy_environment(),
            dummy_scenario(),
            etx,
            &clients,
            &mut handle,
        )
        .await;

        assert_eq!(
            execution_statuses(&handle.status_updates),
            vec![Status::Provisioning, Status::Unrunnable],
            "expected Provisioning then Failed"
        );
    }

    #[tokio::test]
    async fn wait_and_update_sets_running_on_workflow_success() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::with_workflow(WatchOutcome::Succeeded);
        let (etx, mut erx) = mpsc::unbounded_channel();

        wait_and_update(&ex, dummy_scenario(), &etx, &clients, &mut handle).await;

        assert_single_execution_status(&handle.status_updates, Status::Running);
        let event = erx.try_recv().expect("expected RunScenario event");
        assert!(
            matches!(event.ty, EventType::RunScenario(_)),
            "expected RunScenario event, got: {:?}",
            event.ty
        );
    }

    #[tokio::test]
    async fn wait_and_update_sets_failed_on_workflow_failure() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::with_workflow(WatchOutcome::Failed("reason".into()));
        let (etx, _erx) = mpsc::unbounded_channel();

        wait_and_update(&ex, dummy_scenario(), &etx, &clients, &mut handle).await;

        assert_eq!(handle.status_updates.len(), 1);
        let TaggedStatusUpdate::Execution(_, ref s) = handle.status_updates[0] else {
            panic!("expected Execution update");
        };
        assert_eq!(s.status, Status::Unrunnable);
        assert!(
            s.message.as_deref().unwrap_or("").contains("reason"),
            "message should contain 'reason', got: {:?}",
            s.message
        );
    }

    #[tokio::test]
    async fn wait_and_update_sets_failed_on_watcher_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::with_workflow(WatchOutcome::WatcherError("watch err".into()));
        let (etx, _erx) = mpsc::unbounded_channel();

        wait_and_update(&ex, dummy_scenario(), &etx, &clients, &mut handle).await;

        assert_single_execution_status(&handle.status_updates, Status::Unrunnable);
    }

    #[tokio::test]
    async fn wait_and_update_sets_failed_on_stream_closed() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::with_workflow(WatchOutcome::StreamClosed);
        let (etx, _erx) = mpsc::unbounded_channel();

        wait_and_update(&ex, dummy_scenario(), &etx, &clients, &mut handle).await;

        assert_eq!(handle.status_updates.len(), 1);
        let TaggedStatusUpdate::Execution(_, ref s) = handle.status_updates[0] else {
            panic!("expected Execution update");
        };
        assert_eq!(s.status, Status::Unrunnable);
        assert!(
            s.message.as_deref().unwrap_or("").contains("stream closed"),
            "message should contain 'stream closed', got: {:?}",
            s.message
        );
    }

    #[tokio::test]
    async fn wait_and_update_deletes_configmap_on_success() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::with_workflow(WatchOutcome::Succeeded);
        let (etx, _erx) = mpsc::unbounded_channel();

        wait_and_update(&ex, dummy_scenario(), &etx, &clients, &mut handle).await;

        assert!(
            clients.delete_configmap_result.lock().unwrap().is_none(),
            "delete_management_configmap must have been called"
        );
    }

    #[tokio::test]
    async fn wait_and_update_deletes_configmap_on_failure() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::with_workflow(WatchOutcome::Failed("reason".into()));
        let (etx, _erx) = mpsc::unbounded_channel();

        wait_and_update(&ex, dummy_scenario(), &etx, &clients, &mut handle).await;

        assert!(
            clients.delete_configmap_result.lock().unwrap().is_none(),
            "delete_management_configmap must have been called"
        );
    }

    #[tokio::test]
    async fn wait_and_update_ignores_configmap_deletion_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::with_workflow_delete_err(WatchOutcome::Succeeded);
        let (etx, _erx) = mpsc::unbounded_channel();

        wait_and_update(&ex, dummy_scenario(), &etx, &clients, &mut handle).await;

        assert_single_execution_status(&handle.status_updates, Status::Running);
    }
}
