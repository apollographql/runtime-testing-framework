use crate::{
    event_loop::{Event, EventType},
    k8s::{
        CLUSTER_API_NAMESPACE, Cluster, ClusterClients, ENVIRONMENT_CONFIG_FILENAME, WatchOutcome,
        env_configmap_name,
    },
};
use rtf_config::formats::{
    DockerComposeEnvironment, DockerScenario, EnvironmentConfig, EnvironmentExecution,
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::error;
use uuid::Uuid;

pub async fn run(
    execution_id: Uuid,
    environment: DockerComposeEnvironment,
    scenario: DockerScenario,
    tx: &UnboundedSender<Event>,
    clients: &ClusterClients,
) -> anyhow::Result<()> {
    clients
        .create_configmap(
            Cluster::Management,
            CLUSTER_API_NAMESPACE,
            env_configmap_name(&execution_id),
            ENVIRONMENT_CONFIG_FILENAME,
            serde_yaml::to_string(&EnvironmentConfig {
                name: Default::default(),
                description: Default::default(),
                variable_definitions: Vec::new(),
                custom_providers: Default::default(),
                execution: EnvironmentExecution::DockerCompose(environment),
            })?,
        )
        .await?;

    // TODO: mark status in DB

    clients.create_argo_workflow(&execution_id).await?;

    // TODO: mark status in DB

    let tx = tx.clone();
    let clients = clients.clone();

    // FIXME: we need a way of checking for and handling image-pull-backoff

    tokio::spawn(async move {
        match clients.wait_for_workflow(&execution_id).await {
            WatchOutcome::Succeeded => {
                // TODO: mark status in DB

                // TODO: handle the error here
                _ = tx.send(Event {
                    execution_id,
                    ty: EventType::RunScenario(scenario),
                });
            }

            outcome => error!(?outcome, "error waiting for argo workflow to complete"),
        }
    });

    Ok(())
}
