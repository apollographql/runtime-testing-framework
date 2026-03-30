use crate::{
    db::{TestExecution, UpdateHandle},
    event_loop::{Error, Event, EventData, Result},
    k8s::{
        self, CONFIG_MAP_NAME_SCENARIO, Cluster, SCENARIO_CONFIG_FILENAME, WatchOutcome,
        scenario_job,
    },
};
use rtf_config::formats::{DockerScenario, ScenarioConfig};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

const SCENARIO_JOB_NAME: &str = "scenario-execution";
const MSG_CREATE_SCENARIO_CM: &str = "creating scenario configmap";
const MSG_JOB_CREATE: &str = "creating scenario job";
const MSG_JOB_WAIT: &str = "waiting for scenario job to complete";

pub(super) async fn try_run<K, H>(
    test_execution: TestExecution,
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
    let namespace = execution_id.to_string();

    let content = serde_yaml::to_string(&ScenarioConfig {
        name: execution_id.to_string(),
        execution: scenario.clone(),
        description: Default::default(),
        variable_definitions: Default::default(),
        custom_providers: Default::default(),
    })
    .unwrap_or_else(|e| panic!("ScenarioConfig failed to serialize: {e}"));

    info!(%execution_id, "creating scenario configmap");
    conn.mark_execution_as_provisioning(&test_execution, MSG_CREATE_SCENARIO_CM.to_string())
        .await;
    clients
        .create_configmap(
            Cluster::Workload,
            &namespace,
            CONFIG_MAP_NAME_SCENARIO,
            SCENARIO_CONFIG_FILENAME,
            content,
        )
        .await
        .map_err(|error| Error::CreateConfigmap {
            kind: "scenario",
            error,
        })?;

    info!(%execution_id, "creating scenario job");
    conn.mark_execution_as_provisioning(&test_execution, MSG_JOB_CREATE.to_string())
        .await;
    clients
        .create_job(
            &namespace,
            SCENARIO_JOB_NAME,
            scenario_job(&execution_id, &scenario),
        )
        .await
        .map_err(|error| Error::CreateJob { error })?;

    conn.mark_execution_as_provisioning(&test_execution, MSG_JOB_WAIT.to_string())
        .await;

    info!(%execution_id, "waiting for scenario job to complete");
    tokio::spawn(async move {
        wait_and_update(&namespace, test_execution, &etx, clients).await;
    });

    Ok(())
}

async fn wait_and_update<K>(
    namespace: &str,
    test_execution: TestExecution,
    etx: &UnboundedSender<Event>,
    clients: K,
) where
    K: k8s::Client,
{
    let execution_id = test_execution.uuid();
    let data = match clients
        .wait_for_job(namespace, &test_execution.uuid())
        .await
    {
        WatchOutcome::Succeeded => {
            info!(%execution_id, "job completed successfully");
            EventData::CleanupNamespace
        }

        WatchOutcome::Failed(reason) => {
            warn!(%execution_id, %reason, "job failed");
            EventData::MarkUnrunnable(WatchOutcome::Failed(reason).to_string())
        }

        outcome => {
            warn!(%execution_id, %outcome, "unable to determine state of job");
            EventData::MarkUnrunnable(outcome.to_string())
        }
    };

    let _ = etx.send(Event {
        test_execution,
        data,
    });
}
