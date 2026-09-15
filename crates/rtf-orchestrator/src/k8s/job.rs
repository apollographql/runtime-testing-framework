use crate::{
    db::TestExecution,
    event_loop::CreateJobConfig,
    k8s::{CLI_BINARY, SCENARIO_RUNNER_CONTAINER, SCENARIO_SA_NAME},
};
use k8s_openapi::{
    api::{
        batch::v1::JobSpec,
        core::v1::{
            Affinity, Container, EnvVar, ExecAction, Lifecycle, LifecycleHandler, PodAffinity,
            PodAffinityTerm, PodAntiAffinity, PodSpec, PodTemplateSpec, Volume, VolumeMount,
        },
    },
    apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement},
};
use kube::api::ObjectMeta;
use rtf_orchestrator_shared::{EXECUTION_ID_LABEL, LOG_COLLECTION_LABEL};
use std::collections::BTreeMap;

const SHARED_DIR_PATH: &str = "/shared";
const TTL_SECONDS_AFTER_FINISHED: i32 = 3600; // cleanup after 1h
const VOLUME_MOUNT_NAME_SHARED: &str = "shared";

/// Build a [JobSpec] that runs a `DockerScenario` under the RTF Orchestrator.
///
/// Layout:
/// - Init container `rtf-resolve` (toolbox image) resolves the scenario config and writes
///   `run.sh` into the shared volume via `rtf-orchestrator-cli prepare-scenario`.
/// - Regular container `scenario-runner` (user image) executes `run.sh`. On exit, the script
///   touches a sentinel file in the shared volume so the `output-collector` running in
///   parallel can detect completion.
/// - Regular container `output-collector` (toolbox image) polls for the sentinel, uploads
///   artifacts, and posts the terminal status via `rtf-orchestrator-cli collect-output`.
pub(crate) fn scenario_job(
    ex: &TestExecution,
    scenario_image: String,
    scenario_command: String,
    config: &CreateJobConfig<'_>,
) -> JobSpec {
    let env = ex.toolbox_env_vars(config.orchestrator_url);

    JobSpec {
        backoff_limit: Some(0), // don't retry failed scenarios
        ttl_seconds_after_finished: Some(TTL_SECONDS_AFTER_FINISHED),
        template: PodTemplateSpec {
            metadata: Some(ObjectMeta {
                labels: Some(BTreeMap::from([
                    (EXECUTION_ID_LABEL.to_owned(), ex.uuid().to_string()),
                    (LOG_COLLECTION_LABEL.to_owned(), "true".to_owned()),
                ])),
                ..Default::default()
            }),
            spec: Some(PodSpec {
                restart_policy: Some("Never".to_owned()),
                init_containers: Some(vec![rtf_resolve_container_spec(
                    scenario_command,
                    config.toolbox_pull_policy,
                    config.toolbox_image,
                    &env,
                )]),
                containers: vec![
                    scenario_run_container_spec(scenario_image),
                    output_collector_container_spec(
                        config.toolbox_pull_policy,
                        config.toolbox_image,
                        config.prometheus_endpoint,
                        &env,
                    ),
                ],
                volumes: Some(scenario_volumes()),
                service_account_name: Some(SCENARIO_SA_NAME.to_owned()),
                affinity: scenario_affinity(config, &ex.uuid().to_string()),
                node_selector: (!config.scenario_node_selector.is_empty())
                    .then(|| config.scenario_node_selector.clone()),
                ..Default::default()
            }),
        },
        ..Default::default()
    }
}

/// The scenario pod's placement relative to other executions
fn scenario_affinity(config: &CreateJobConfig<'_>, execution_id: &str) -> Option<Affinity> {
    if !config.exclusive_nodes {
        return None;
    }

    Some(if config.scenario_node_selector.is_empty() {
        co_located_with_environment(execution_id)
    } else {
        exclusive_from_other_executions(execution_id)
    })
}

/// Require the scenario pod to land on the same node as its own execution's environment pods
fn co_located_with_environment(execution_id: &str) -> Affinity {
    Affinity {
        pod_affinity: Some(PodAffinity {
            required_during_scheduling_ignored_during_execution: Some(vec![PodAffinityTerm {
                label_selector: Some(LabelSelector {
                    match_expressions: Some(vec![LabelSelectorRequirement {
                        key: EXECUTION_ID_LABEL.to_owned(),
                        operator: "In".to_owned(),
                        values: Some(vec![execution_id.to_owned()]),
                    }]),
                    ..Default::default()
                }),
                topology_key: "kubernetes.io/hostname".to_owned(),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Bar scenario pods of other executions from sharing a node in the dedicated pool.
fn exclusive_from_other_executions(execution_id: &str) -> Affinity {
    Affinity {
        pod_anti_affinity: Some(PodAntiAffinity {
            required_during_scheduling_ignored_during_execution: Some(vec![PodAffinityTerm {
                namespace_selector: Some(LabelSelector::default()),
                label_selector: Some(LabelSelector {
                    match_expressions: Some(vec![
                        LabelSelectorRequirement {
                            key: EXECUTION_ID_LABEL.to_owned(),
                            operator: "Exists".to_owned(),
                            values: None,
                        },
                        LabelSelectorRequirement {
                            key: EXECUTION_ID_LABEL.to_owned(),
                            operator: "NotIn".to_owned(),
                            values: Some(vec![execution_id.to_owned()]),
                        },
                    ]),
                    ..Default::default()
                }),
                topology_key: "kubernetes.io/hostname".to_owned(),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn rtf_resolve_container_spec(
    scenario_command: String,
    toolbox_pull_policy: &str,
    toolbox_image: &str,
    env: &[EnvVar],
) -> Container {
    Container {
        name: "rtf-resolve".to_owned(),
        image: Some(toolbox_image.to_owned()),
        image_pull_policy: Some(toolbox_pull_policy.to_owned()),
        command: Some(vec![CLI_BINARY.to_owned()]),
        args: Some(vec![
            "prepare-scenario".into(),
            "--shared-dir".into(),
            SHARED_DIR_PATH.into(),
            "--command".into(),
            scenario_command,
        ]),
        env: Some(env.to_vec()),
        volume_mounts: Some(vec![VolumeMount {
            name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
            mount_path: SHARED_DIR_PATH.to_owned(),
            read_only: Some(false),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

fn scenario_run_container_spec(scenario_image: String) -> Container {
    Container {
        name: SCENARIO_RUNNER_CONTAINER.to_owned(),
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

fn output_collector_container_spec(
    toolbox_pull_policy: &str,
    toolbox_image: &str,
    prometheus_endpoint: &str,
    env: &[EnvVar],
) -> Container {
    Container {
        name: SCENARIO_SA_NAME.to_owned(),
        image: Some(toolbox_image.to_owned()),
        image_pull_policy: Some(toolbox_pull_policy.to_owned()),
        command: Some(vec![CLI_BINARY.to_owned()]),
        args: Some(vec![
            "collect-output".into(),
            "--shared-dir".into(),
            SHARED_DIR_PATH.into(),
            "--prometheus-endpoint".into(),
            prometheus_endpoint.into(),
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
    vec![Volume {
        name: VOLUME_MOUNT_NAME_SHARED.to_owned(),
        empty_dir: Some(Default::default()),
        ..Default::default()
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClusterRoles;

    /// Returns the built pod spec alongside the stub execution's id, since `create_stub`
    /// generates a random uuid the affinity assertions need to compare against.
    fn build(
        exclusive_nodes: bool,
        scenario_node_selector: &BTreeMap<String, String>,
    ) -> (PodSpec, String) {
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let execution_id = ex.uuid().to_string();
        let cluster_roles = ClusterRoles {
            cluster_read: String::new(),
            namespace_read: String::new(),
            namespace_write: String::new(),
        };
        let config = CreateJobConfig {
            orchestrator_url: "http://localhost:8035",
            prometheus_endpoint: "http://prometheus:9090",
            toolbox_pull_policy: "IfNotPresent",
            toolbox_image: "rtf-toolbox:edge",
            cluster_roles: &cluster_roles,
            allow_namespace_write: false,
            exclusive_nodes,
            scenario_node_selector,
        };
        let job = scenario_job(&ex, "image".into(), "cmd".into(), &config);
        let pod_spec = job
            .template
            .spec
            .expect("scenario job should have a pod spec");

        (pod_spec, execution_id)
    }

    #[test]
    fn omits_node_selector_when_scenario_node_selector_is_empty() {
        let (pod_spec, _) = build(false, &BTreeMap::new());
        assert!(pod_spec.node_selector.is_none());
    }

    #[test]
    fn sets_node_selector_when_scenario_node_selector_is_configured() {
        let selector = BTreeMap::from([("pool".to_string(), "load-generators".to_string())]);
        let (pod_spec, _) = build(false, &selector);

        assert_eq!(pod_spec.node_selector, Some(selector));
    }

    #[test]
    fn omits_affinity_when_exclusive_nodes_is_disabled() {
        let (pod_spec, _) = build(false, &BTreeMap::new());
        assert!(pod_spec.affinity.is_none());
    }

    #[test]
    fn omits_affinity_when_scenario_node_selector_is_configured_without_exclusive_nodes() {
        let selector = BTreeMap::from([("pool".to_string(), "load-generators".to_string())]);
        let (pod_spec, _) = build(false, &selector);

        assert!(pod_spec.affinity.is_none());
    }

    #[test]
    fn requires_exclusivity_from_other_executions_when_exclusive_nodes_and_selector_configured() {
        let selector = BTreeMap::from([("pool".to_string(), "load-generators".to_string())]);
        let (pod_spec, execution_id) = build(true, &selector);
        let terms = pod_spec
            .affinity
            .expect("expected an affinity block")
            .pod_anti_affinity
            .expect("expected a podAntiAffinity block")
            .required_during_scheduling_ignored_during_execution
            .expect("expected required terms");

        assert_eq!(terms.len(), 1, "expected exactly one required term");
        let term = &terms[0];

        assert_eq!(term.topology_key, "kubernetes.io/hostname");
        assert!(
            term.namespace_selector.is_some(),
            "term must widen across namespaces, since every execution has its own"
        );

        let expressions = term
            .label_selector
            .as_ref()
            .and_then(|s| s.match_expressions.as_ref())
            .expect("expected match expressions");
        assert_eq!(expressions.len(), 2);
        assert_eq!(expressions[0].key, EXECUTION_ID_LABEL);
        assert_eq!(expressions[0].operator, "Exists");
        assert_eq!(expressions[1].key, EXECUTION_ID_LABEL);
        assert_eq!(expressions[1].operator, "NotIn");
        assert_eq!(expressions[1].values, Some(vec![execution_id]));
    }

    #[test]
    fn requires_co_location_with_own_execution_when_exclusive_nodes_and_no_selector() {
        let (pod_spec, execution_id) = build(true, &BTreeMap::new());
        let terms = pod_spec
            .affinity
            .expect("expected an affinity block")
            .pod_affinity
            .expect("expected a podAffinity block")
            .required_during_scheduling_ignored_during_execution
            .expect("expected required terms");

        assert_eq!(terms.len(), 1, "expected exactly one required term");
        let term = &terms[0];

        assert_eq!(term.topology_key, "kubernetes.io/hostname");

        let expressions = term
            .label_selector
            .as_ref()
            .and_then(|s| s.match_expressions.as_ref())
            .expect("expected match expressions");
        assert_eq!(expressions.len(), 1);
        assert_eq!(expressions[0].key, EXECUTION_ID_LABEL);
        assert_eq!(expressions[0].operator, "In");
        assert_eq!(expressions[0].values, Some(vec![execution_id]));
    }
}
