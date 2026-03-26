use crate::{
    conn,
    db::{Status, TestExecution, UpdateHandle},
    k8s::{self, CLUSTER_API_NAMESPACE, Cluster, ENVIRONMENT_CONFIG_FILENAME, env_configmap_name},
};
use rtf_config::formats::{DockerComposeEnvironment, DockerScenario, EnvironmentConfig};
use tracing::error;

pub async fn run<K>(
    test_execution: TestExecution,
    environment: DockerComposeEnvironment,
    _scenario: DockerScenario,
    clients: &K,
) where
    K: k8s::Client,
{
    let execution_id = test_execution.uuid();
    let result: crate::Result<()> = async {
        try_run(test_execution, environment, clients, conn!()).await;
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
    clients: &K,
    db: &mut H,
) where
    K: k8s::Client,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{MockUpdateHandle, TaggedStatusUpdate},
        k8s::mock_client::MockClient,
    };

    fn dummy_environment() -> DockerComposeEnvironment {
        DockerComposeEnvironment {
            project_name: None,
            compose_files: vec![],
            file_providers: vec![],
            env_vars: Default::default(),
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

    #[tokio::test]
    async fn try_run_sets_provisioning_on_success() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::ok();

        try_run(ex, dummy_environment(), &clients, &mut handle).await;

        assert_single_execution_status(&handle.status_updates, Status::Provisioning);
    }

    #[tokio::test]
    async fn try_run_sets_failed_on_configmap_error() {
        let ex = TestExecution::create_stub(1, 1, "test");
        let mut handle = MockUpdateHandle::with_execution(ex.clone());
        let clients = MockClient::configmap_err();

        try_run(ex, dummy_environment(), &clients, &mut handle).await;

        assert_single_execution_status(&handle.status_updates, Status::Unrunnable);
        assert!(
            clients.create_configmap_result.lock().unwrap().is_none(),
            "create_configmap must have been called"
        );
    }
}
