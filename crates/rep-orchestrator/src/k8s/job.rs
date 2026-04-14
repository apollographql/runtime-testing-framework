use crate::k8s::{EXECUTION_ID_LABEL, TOOLBOX_IMAGE};
use k8s_openapi::api::{
    batch::v1::JobSpec,
    core::v1::{
        ConfigMapVolumeSource, Container, ExecAction, Lifecycle, LifecycleHandler, PodSpec,
        PodTemplateSpec, Volume, VolumeMount,
    },
};
use kube::api::ObjectMeta;
use rtf_config::formats::DockerScenario;
use std::collections::BTreeMap;
use uuid::Uuid;

pub const CONFIG_MAP_NAME_SCENARIO: &str = "rtf-scenario-config";
const SHARED_DIR_PATH: &str = "/shared";
const TTL_SECONDS_AFTER_FINISHED: i32 = 3600; // cleanup after 1h
const VOLUME_MOUNT_NAME_CONFIG: &str = "scenario-config";
const VOLUME_MOUNT_NAME_SHARED: &str = "shared";

// FIXME: these scripts need to be moved out to Rust code within the toolbox once we've settled on
// the implementation

const OUTPUT_COLLECTOR_SCRIPT: &str = r#"
echo "Waiting for scenario to complete..."
while [ ! -f /shared/scenario-exited ]; do
  sleep 5
done

echo "Scenario finished. Showing output..."
ls -laR /shared/
cat /shared/output/output.log
"#;

const RESOLVE_SCRIPT: &str = r#"
set -e

echo "Resolving scenario..."
rtf resolve scenario /scenario/scenario.yaml --outdir /shared/providers

echo "Writing out user scenario script..."
cat << 'EOF' > /shared/scenario.sh
#!/bin/sh
set -ex

echo "Sourcing RTF env vars"
. /shared/providers/scenario.env

export OUTDIR=/shared/output
export RTF_OUTPUT="$OUTDIR/RTF_OUTPUT"

echo "Running user specified scenario..."
__SCENARIO_COMMAND__
EOF

echo "Writing out run script..."
cat << 'EOF' > /shared/run.sh
#!/bin/sh
set -ex

trap 'touch /shared/scenario-exited' EXIT
echo "Exit trap installed"

mkdir -p /shared/output

echo "Running test scenario"
{ /shared/scenario.sh 2>&1; echo $? > /shared/scenario_exit_status; } |
  tee /shared/output/output.log
EOF

chmod -R 777 /shared/scenario.sh
chmod -R 777 /shared/run.sh

echo "Contents of scenario script"
cat /shared/scenario.sh

ls -laR /shared/providers/
"#;

/// Create a new [JobSpec] for the given [DockerScenario].
pub fn scenario_job(execution_id: &Uuid, scenario: &DockerScenario) -> JobSpec {
    JobSpec {
        backoff_limit: Some(0), // don't retry failed scenarios
        ttl_seconds_after_finished: Some(TTL_SECONDS_AFTER_FINISHED),
        template: PodTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: Some(BTreeMap::from([(
                    EXECUTION_ID_LABEL.to_owned(),
                    execution_id.to_string(),
                )])),
                ..Default::default()
            }),
            spec: Some(PodSpec {
                restart_policy: Some("Never".to_owned()),
                init_containers: Some(vec![init_container_spec(scenario)]),
                containers: vec![
                    scenario_run_container_spec(scenario),
                    output_collector_container_spec(),
                ],
                volumes: Some(scenario_volumes()),
                ..Default::default()
            }),
        },
        ..Default::default()
    }
}

fn init_container_spec(scenario: &DockerScenario) -> Container {
    Container {
        name: "rtf-resolve".to_owned(),
        image: Some(TOOLBOX_IMAGE.to_owned()),
        command: Some(vec!["/bin/sh".to_owned(), "-c".to_owned()]),
        args: Some(vec![
            RESOLVE_SCRIPT.replace("__SCENARIO_COMMAND__", &scenario.command()),
        ]),
        volume_mounts: Some(vec![
            VolumeMount {
                name: VOLUME_MOUNT_NAME_CONFIG.to_owned(),
                mount_path: "/scenario".to_owned(),
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

fn scenario_run_container_spec(scenario: &DockerScenario) -> Container {
    Container {
        name: "scenario-runner".to_owned(),
        image: Some(scenario.docker_image()),
        image_pull_policy: Some("Always".to_string()),
        command: Some(vec!["/bin/sh".to_owned(), "/shared/run.sh".to_owned()]),
        working_dir: Some(SHARED_DIR_PATH.to_owned()),
        volume_mounts: Some(vec![VolumeMount {
            name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
            mount_path: SHARED_DIR_PATH.to_owned(),
            read_only: Some(false),
            ..Default::default()
        }]),
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

fn output_collector_container_spec() -> Container {
    Container {
        name: "output-collector".to_owned(),
        image: Some(TOOLBOX_IMAGE.to_owned()),
        command: Some(vec!["/bin/sh".to_owned(), "-c".to_owned()]),
        args: Some(vec![OUTPUT_COLLECTOR_SCRIPT.to_owned()]),
        volume_mounts: Some(vec![
            VolumeMount {
                name: VOLUME_MOUNT_NAME_CONFIG.to_owned(),
                mount_path: "/scenario".to_owned(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
                mount_path: "/shared".to_owned(),
                read_only: Some(true),
                ..Default::default()
            },
        ]),
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
