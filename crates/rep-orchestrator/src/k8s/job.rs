use crate::{
    db::TestExecution,
    k8s::{CLI_BINARY, EXECUTION_ID_LABEL, TOOLBOX_IMAGE},
};
use k8s_openapi::api::{
    batch::v1::JobSpec,
    core::v1::{
        ConfigMapVolumeSource, Container, EnvVar, ExecAction, Lifecycle, LifecycleHandler, PodSpec,
        PodTemplateSpec, Volume, VolumeMount,
    },
};
use kube::api::ObjectMeta;
use std::collections::BTreeMap;

pub const CONFIG_MAP_NAME_SCENARIO: &str = "rtf-scenario-config";

const SHARED_DIR_PATH: &str = "/shared";
const SCENARIO_CONFIG_DIR: &str = "/scenario";
const SCENARIO_CONFIG_PATH: &str = "/scenario/scenario.yaml";
const TTL_SECONDS_AFTER_FINISHED: i32 = 3600; // cleanup after 1h

const VOLUME_MOUNT_NAME_CONFIG: &str = "scenario-config";
const VOLUME_MOUNT_NAME_SHARED: &str = "shared";

/// Build a [JobSpec] that runs a `DockerScenario` under REP.
///
/// Layout:
/// - Init container `rtf-resolve` (toolbox image) resolves the scenario config and writes
///   `run.sh` into the shared volume via `rep-orchestrator-cli prepare-scenario`.
/// - Regular container `scenario-runner` (user image) executes `run.sh`. On exit, the script
///   touches a sentinel file in the shared volume so the `output-collector` running in
///   parallel can detect completion.
/// - Regular container `output-collector` (toolbox image) polls for the sentinel, uploads
///   artifacts, and posts the terminal status via `rep-orchestrator-cli collect-output`.
pub fn scenario_job(
    ex: &TestExecution,
    scenario_image: String,
    scenario_command: String,
    orchestrator_url: &str,
) -> JobSpec {
    let env = ex.toolbox_env_vars(orchestrator_url);

    JobSpec {
        backoff_limit: Some(0), // don't retry failed scenarios
        ttl_seconds_after_finished: Some(TTL_SECONDS_AFTER_FINISHED),
        template: PodTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: Some(BTreeMap::from([(
                    EXECUTION_ID_LABEL.to_owned(),
                    ex.uuid().to_string(),
                )])),
                ..Default::default()
            }),
            spec: Some(PodSpec {
                restart_policy: Some("Never".to_owned()),
                init_containers: Some(vec![rtf_resolve_container_spec(scenario_command, &env)]),
                containers: vec![
                    scenario_run_container_spec(scenario_image),
                    output_collector_container_spec(&env),
                ],
                volumes: Some(scenario_volumes()),
                ..Default::default()
            }),
        },
        ..Default::default()
    }
}

fn rtf_resolve_container_spec(scenario_command: String, env: &[EnvVar]) -> Container {
    Container {
        name: "rtf-resolve".to_owned(),
        image: Some(TOOLBOX_IMAGE.to_owned()),
        command: Some(vec![CLI_BINARY.to_owned()]),
        args: Some(vec![
            "prepare-scenario".into(),
            "--scenario".into(),
            SCENARIO_CONFIG_PATH.into(),
            "--shared-dir".into(),
            SHARED_DIR_PATH.into(),
            "--command".into(),
            scenario_command,
        ]),
        env: Some(env.to_vec()),
        volume_mounts: Some(vec![
            VolumeMount {
                name: VOLUME_MOUNT_NAME_CONFIG.to_owned(),
                mount_path: SCENARIO_CONFIG_DIR.to_owned(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
                mount_path: SHARED_DIR_PATH.to_owned(),
                read_only: Some(false),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

fn scenario_run_container_spec(scenario_image: String) -> Container {
    Container {
        name: "scenario-runner".to_owned(),
        image: Some(scenario_image),
        image_pull_policy: Some("Always".to_string()),
        command: Some(vec!["/bin/sh".to_owned(), "/shared/run.sh".to_owned()]),
        working_dir: Some(SHARED_DIR_PATH.to_owned()),
        volume_mounts: Some(vec![VolumeMount {
            name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
            mount_path: SHARED_DIR_PATH.to_owned(),
            read_only: Some(false),
            ..Default::default()
        }]),
        // Flush writes on the shared volume before the container is torn down, so the
        // output-collector can observe a consistent final state (sentinel, exit-status file,
        // output.log).
        lifecycle: Some(Lifecycle {
            pre_stop: Some(LifecycleHandler {
                exec: Some(ExecAction {
                    command: Some(vec![
                        "/bin/sh".to_owned(),
                        "-c".to_owned(),
                        "sync".to_owned(),
                    ]),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn output_collector_container_spec(env: &[EnvVar]) -> Container {
    Container {
        name: "output-collector".to_owned(),
        image: Some(TOOLBOX_IMAGE.to_owned()),
        command: Some(vec![CLI_BINARY.to_owned()]),
        args: Some(vec![
            "collect-output".into(),
            "--shared-dir".into(),
            SHARED_DIR_PATH.into(),
        ]),
        env: Some(env.to_vec()),
        volume_mounts: Some(vec![VolumeMount {
            name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
            mount_path: SHARED_DIR_PATH.to_owned(),
            // We need write access to create the output zip file
            read_only: Some(false),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

fn scenario_volumes() -> Vec<Volume> {
    vec![
        Volume {
            name: VOLUME_MOUNT_NAME_CONFIG.to_owned(),
            config_map: Some(ConfigMapVolumeSource {
                name: CONFIG_MAP_NAME_SCENARIO.to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        },
        Volume {
            name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
            empty_dir: Some(Default::default()),
            ..Default::default()
        },
    ]
}
