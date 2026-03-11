use crate::{
    event_loop::{Event, EventType},
    k8s::{
        CONFIG_MAP_NAME_SCENARIO, Cluster, ClusterClients, SCENARIO_CONFIG_FILENAME, WatchOutcome,
        scenario_job,
    },
};
use rtf_config::formats::{DockerScenario, ScenarioCommand, ScenarioConfig};
use tokio::sync::mpsc::UnboundedSender;
use tracing::error;
use uuid::Uuid;

const SCENARIO_JOB_NAME: &str = "scenario-execution";

pub async fn run(
    execution_id: Uuid,
    scenario: DockerScenario,
    tx: &UnboundedSender<Event>,
    clients: &ClusterClients,
) -> anyhow::Result<()> {
    let ns = execution_id.to_string();
    let spec = scenario_job(&execution_id, &scenario);

    clients
        .create_configmap(
            Cluster::Workload,
            &ns,
            CONFIG_MAP_NAME_SCENARIO,
            SCENARIO_CONFIG_FILENAME,
            serde_yaml::to_string(&ScenarioConfig {
                name: Default::default(),
                description: Default::default(),
                variable_definitions: Default::default(),
                custom_providers: Default::default(),
                command: ScenarioCommand::Docker(scenario),
            })?,
        )
        .await?;

    // TODO: mark status in DB

    clients
        .create_job(
            Cluster::Workload,
            &execution_id.to_string(),
            SCENARIO_JOB_NAME,
            spec,
        )
        .await?;

    // TODO: mark status in DB

    let tx = tx.clone();
    let clients = clients.clone();

    tokio::spawn(async move {
        match clients
            .wait_for_job(Cluster::Workload, &ns, SCENARIO_JOB_NAME)
            .await
        {
            WatchOutcome::Succeeded => {
                // TODO: mark status in DB

                // TODO: handle the error here
                _ = tx.send(Event {
                    execution_id,
                    ty: EventType::CleanupNamespace,
                });
            }

            outcome => error!(?outcome, "error waiting for scenario job to complete"),
        }
    });

    Ok(())
}
