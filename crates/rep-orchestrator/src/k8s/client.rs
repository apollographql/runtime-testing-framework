use crate::k8s::{
    CLUSTER_API_NAMESPACE, Error, FullClient, ManagementClient, OUTPUT_COLLECTOR, Result,
    SCENARIO_RUNNER_CONTAINER, WatchOutcome, Workflow, WorkflowSpec, WorkloadClient, workflow_name,
};
use chrono::{DateTime, Duration, Utc};
use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::{Namespace, Pod, ServiceAccount},
    rbac::v1::{ClusterRole, ClusterRoleBinding, PolicyRule, Role, RoleBinding, RoleRef, Subject},
};
use kube::{
    Client, Config, Resource,
    api::{Api, ListParams, ObjectMeta, Patch, PatchParams},
    config::{KubeConfigOptions, Kubeconfig},
    core::NamespaceResourceScope,
};
use rep_orchestrator_shared::EXECUTION_ID_LABEL;
use std::collections::BTreeMap;
use tokio::time::sleep;
use tracing::{error, warn};
use uuid::Uuid;

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const CLIENT_REFRESH_MARGIN: Duration = Duration::seconds(40 * 60);
const RETRY_WINDOW: Duration = Duration::seconds(15 * 60);

// Container waiting reasons that indicate a pod is permanently stuck and will never produce an
// exit code. These surface as `containerStatuses[].state.waiting.reason` in the pod status.
//
// Unlike transient failures (OOMKilled, CrashLoopBackOff, Error), which produce an exit code and
// propagate to the parent Job/Workflow as Failed, these states leave the container in Waiting
// indefinitely — the Job/Workflow never transitions, and the execution hangs.
//
// The reason strings are kubelet implementation details, not part of the Kubernetes API contract.
// Image-related reasons originate in pkg/kubelet/images/types.go [1]; container creation reasons
// in pkg/kubelet/kuberuntime/kuberuntime_container.go [2]. The official pod lifecycle docs
// describe the Waiting state but do not enumerate possible reason values [3].
//
// This list covers known cases but is not exhaustive — other Waiting reasons may exist that
// would also cause executions to hang.
//
// [1] https://github.com/kubernetes/kubernetes/blob/cec8f06d2ce283d1563d37849619887aa2b11c9f/pkg/kubelet/images/types.go
// [2] https://github.com/kubernetes/kubernetes/blob/cec8f06d2ce283d1563d37849619887aa2b11c9f/pkg/kubelet/kuberuntime/kuberuntime_container.go
// [3] https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#container-states
const UNRUNNABLE_REASONS: &[&str] = &[
    "ImagePullBackOff",
    "ErrImagePull",
    "ErrImageNeverPull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
];

/// Typestate marker for a [ClusterClients] slot that has not been populated and is therefore
/// unusable from this instance.
#[derive(Clone)]
pub struct Unavailable;

/// Typestate marker for a [ClusterClients] slot that holds an initialised [Client]
/// for the management cluster.
/// The underlying [Client] does not need to be rebuilt for the management cluster
/// as the `kubeconfig` tokens are refreshable through kube client
#[derive(Clone)]
pub struct AvailableManagement(Client);

/// Typestate marker for a [ClusterClients] slot that holds an initialised [Client] for workload
/// clusters.
/// The underlying [Client] contains a token that cannot be refreshed so this needs to rebuild the
/// underlying [Client] from disk.
#[derive(Clone)]
pub struct AvailableWorkload {
    client: Client,
    workload_path: String,
    workload_context: String,
    last_refresh: DateTime<Utc>,
}

impl AvailableWorkload {
    async fn new(workload_path: &str, workload_context: &str) -> Result<Self> {
        let kfg = Kubeconfig::read_from(workload_path)?;
        let client = client_for_context(kfg, workload_context).await?;

        Ok(Self {
            client,
            workload_path: workload_path.to_owned(),
            workload_context: workload_context.to_owned(),
            last_refresh: Utc::now(),
        })
    }

    async fn refresh(&self) -> Result<Self> {
        Self::new(&self.workload_path, &self.workload_context).await
    }
}

#[derive(Clone)]
pub struct ClusterClients<M, W> {
    management: M,
    workload: W,
}

impl ClusterClients<AvailableManagement, Unavailable> {
    /// Construct a client that can only interact with the management cluster.
    pub async fn try_new_management() -> Result<Self> {
        let management = Client::try_from(Config::incluster_env()?)?;

        Ok(Self {
            management: AvailableManagement(management),
            workload: Unavailable,
        })
    }
}

impl ClusterClients<Unavailable, AvailableWorkload> {
    /// Construct a client that can only interact with the workload cluster.
    pub async fn try_new_workload(workload_path: &str, workload_context: &str) -> Result<Self> {
        Ok(Self {
            management: Unavailable,
            workload: AvailableWorkload::new(workload_path, workload_context).await?,
        })
    }
}

impl ClusterClients<AvailableManagement, AvailableWorkload> {
    /// Construct a client that can interact with both clusters.
    pub async fn try_new_full(workload_path: &str, workload_context: &str) -> Result<Self> {
        let management = Client::try_from(Config::incluster_env()?)?;

        Ok(Self {
            management: AvailableManagement(management),
            workload: AvailableWorkload::new(workload_path, workload_context).await?,
        })
    }
}

impl<W> ClusterClients<AvailableManagement, W> {
    fn management_api<K>(&self, ns: &str) -> Api<K>
    where
        K: Resource<Scope = NamespaceResourceScope>,
        <K as Resource>::DynamicType: Default,
    {
        Api::namespaced(self.management.0.clone(), ns)
    }
}

impl<M> ClusterClients<M, AvailableWorkload> {
    async fn workload_api<K>(&mut self, ns: &str) -> Result<Api<K>>
    where
        K: Resource<Scope = NamespaceResourceScope>,
        <K as Resource>::DynamicType: Default,
    {
        let client = self.workload_client().await?;

        Ok(Api::namespaced(client, ns))
    }

    async fn workload_client(&mut self) -> Result<Client> {
        self.refresh_workload_if_due().await?;

        Ok(self.workload.client.clone())
    }

    /// Rebuild the workload client from disk if `last_refresh` is older than
    /// [CLIENT_REFRESH_MARGIN]
    async fn refresh_workload_if_due(&mut self) -> Result<()> {
        if Utc::now() - self.workload.last_refresh > CLIENT_REFRESH_MARGIN {
            self.refresh_workload_now().await?;
        }

        Ok(())
    }

    async fn refresh_workload_now(&mut self) -> Result<()> {
        self.workload = self.workload.refresh().await?;

        Ok(())
    }

    async fn create_output_collector_rbac(&mut self, ns: &str) -> Result<()> {
        let pp = PatchParams::apply("rep-orchestrator");

        let sa_api: Api<ServiceAccount> = self.workload_api(ns).await?;
        sa_api
            .patch(
                OUTPUT_COLLECTOR,
                &pp,
                &Patch::Apply(&ServiceAccount {
                    metadata: ObjectMeta {
                        name: Some(OUTPUT_COLLECTOR.to_owned()),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            .await?;

        let role_api: Api<Role> = self.workload_api(ns).await?;
        role_api
            .patch(
                OUTPUT_COLLECTOR,
                &pp,
                &Patch::Apply(&Role {
                    metadata: ObjectMeta {
                        name: Some(OUTPUT_COLLECTOR.to_owned()),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    },
                    rules: Some(vec![
                        PolicyRule {
                            api_groups: Some(vec!["".to_owned()]),
                            resources: Some(vec!["pods".to_owned()]),
                            verbs: vec!["list".to_owned()],
                            ..Default::default()
                        },
                        PolicyRule {
                            api_groups: Some(vec!["".to_owned()]),
                            resources: Some(vec!["pods/log".to_owned()]),
                            verbs: vec!["get".to_owned()],
                            ..Default::default()
                        },
                        PolicyRule {
                            api_groups: Some(vec!["".to_owned()]),
                            resources: Some(vec!["events".to_owned()]),
                            verbs: vec!["list".to_owned()],
                            ..Default::default()
                        },
                        PolicyRule {
                            api_groups: Some(vec!["batch".to_owned()]),
                            resources: Some(vec!["jobs".to_owned()]),
                            verbs: vec!["get".to_owned()],
                            ..Default::default()
                        },
                    ]),
                }),
            )
            .await?;

        let rb_api: Api<RoleBinding> = self.workload_api(ns).await?;
        rb_api
            .patch(
                OUTPUT_COLLECTOR,
                &pp,
                &Patch::Apply(&RoleBinding {
                    metadata: ObjectMeta {
                        name: Some(OUTPUT_COLLECTOR.to_owned()),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    },
                    role_ref: RoleRef {
                        api_group: "rbac.authorization.k8s.io".to_owned(),
                        kind: "Role".to_owned(),
                        name: OUTPUT_COLLECTOR.to_owned(),
                    },
                    subjects: Some(vec![Subject {
                        kind: "ServiceAccount".to_owned(),
                        name: OUTPUT_COLLECTOR.to_owned(),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    }]),
                }),
            )
            .await?;

        let cr_api: Api<ClusterRole> = Api::all(self.workload_client().await?);
        cr_api
            .patch(
                OUTPUT_COLLECTOR,
                &pp,
                &Patch::Apply(&ClusterRole {
                    metadata: ObjectMeta {
                        name: Some(OUTPUT_COLLECTOR.to_owned()),
                        ..Default::default()
                    },
                    rules: Some(vec![PolicyRule {
                        api_groups: Some(vec!["".to_owned()]),
                        resources: Some(vec!["nodes/proxy".to_owned()]),
                        verbs: vec!["get".to_owned()],
                        ..Default::default()
                    }]),
                    aggregation_rule: None,
                }),
            )
            .await?;

        let crb_name = format!("{OUTPUT_COLLECTOR}-{ns}");
        let crb_api: Api<ClusterRoleBinding> = Api::all(self.workload_client().await?);
        crb_api
            .patch(
                &crb_name,
                &pp,
                &Patch::Apply(&ClusterRoleBinding {
                    metadata: ObjectMeta {
                        name: Some(crb_name.clone()),
                        ..Default::default()
                    },
                    role_ref: RoleRef {
                        api_group: "rbac.authorization.k8s.io".to_owned(),
                        kind: "ClusterRole".to_owned(),
                        name: OUTPUT_COLLECTOR.to_owned(),
                    },
                    subjects: Some(vec![Subject {
                        kind: "ServiceAccount".to_owned(),
                        name: OUTPUT_COLLECTOR.to_owned(),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    }]),
                }),
            )
            .await?;

        Ok(())
    }
}

async fn client_for_context(kfg: Kubeconfig, context: &str) -> Result<Client> {
    let client = Client::try_from(
        Config::from_custom_kubeconfig(
            kfg,
            &KubeConfigOptions {
                context: Some(context.into()),
                ..Default::default()
            },
        )
        .await?,
    )?;

    Ok(client)
}

impl<W: Clone + Send + Sync + 'static> ManagementClient for ClusterClients<AvailableManagement, W> {
    async fn create_argo_workflow(
        &self,
        execution_id: &Uuid,
        spec: WorkflowSpec,
    ) -> Result<Workflow> {
        let wf = self
            .management_api(CLUSTER_API_NAMESPACE)
            .create(
                &Default::default(),
                &Workflow {
                    metadata: ObjectMeta {
                        name: Some(workflow_name(execution_id)),
                        namespace: Some(CLUSTER_API_NAMESPACE.to_owned()),
                        labels: Some(BTreeMap::from([(
                            EXECUTION_ID_LABEL.to_owned(),
                            execution_id.to_string(),
                        )])),
                        ..Default::default()
                    },
                    spec,
                    ..Default::default()
                },
            )
            .await?;

        Ok(wf)
    }
}

impl<M: Clone + Send + Sync + 'static> WorkloadClient for ClusterClients<M, AvailableWorkload> {
    async fn create_job(
        &mut self,
        ns: &str,
        name: &str,
        execution_id: &Uuid,
        spec: JobSpec,
    ) -> Result<Job> {
        self.create_output_collector_rbac(ns).await?;

        let job = self
            .workload_api(ns)
            .await?
            .create(
                &Default::default(),
                &Job {
                    metadata: ObjectMeta {
                        name: Some(name.to_owned()),
                        namespace: Some(ns.to_owned()),
                        labels: Some(BTreeMap::from([(
                            EXECUTION_ID_LABEL.to_owned(),
                            execution_id.to_string(),
                        )])),
                        ..Default::default()
                    },
                    spec: Some(spec),
                    ..Default::default()
                },
            )
            .await?;

        Ok(job)
    }

    async fn wait_for_job(&mut self, ns: &str, execution_id: &Uuid) -> WatchOutcome {
        let labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let params = ListParams::default().labels(&labels);
        // Used to keep track of when the first client failure was detected so a maximum
        // retry window is not exceeded
        let mut first_failure: Option<DateTime<Utc>> = None;

        loop {
            sleep(POLL_INTERVAL).await;

            let result = async {
                let job_api: Api<Job> = self.workload_api(ns).await?;
                let pod_api: Api<Pod> = self.workload_api(ns).await?;
                poll_job_and_pods(&job_api, &pod_api, &params).await
            }
            .await;

            match classify_poll_result(result, &mut first_failure) {
                PollDecision::Terminal(outcome) => return outcome,
                PollDecision::KeepWaiting => {}
                PollDecision::RetryAfterRefresh => {
                    let _ = self.refresh_workload_now().await;
                }
            }
        }
    }

    async fn delete_workload_namespace(&mut self, ns: &str) -> Result<()> {
        let crb_api: Api<ClusterRoleBinding> = Api::all(self.workload_client().await?);
        if let Err(e) = crb_api
            .delete(&format!("{OUTPUT_COLLECTOR}-{ns}"), &Default::default())
            .await
        {
            warn!("failed to delete ClusterRoleBinding {OUTPUT_COLLECTOR}-{ns}: {e}");
        }

        let api: Api<Namespace> = Api::all(self.workload_client().await?);
        api.delete(ns, &Default::default()).await?;

        Ok(())
    }
}

impl FullClient for ClusterClients<AvailableManagement, AvailableWorkload> {
    async fn wait_for_workflow(&mut self, execution_id: &Uuid) -> WatchOutcome {
        let wf_labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let wf_params = ListParams::default().labels(&wf_labels);

        // Argo workflow pods aren't handled particularly well by Argo natively, so we watch them
        // directly to catch a stuck pod the workflow status itself won't report.
        let wf_pod_labels = format!("workflows.argoproj.io/workflow=provision-env-{execution_id}");
        let wf_pod_params = ListParams::default().labels(&wf_pod_labels);

        let workload_ns = execution_id.to_string();
        let workload_pod_params = ListParams::default();
        // Used to keep track of when the first client failure was detected so a maximum
        // retry window is not exceeded
        let mut first_failure: Option<DateTime<Utc>> = None;

        loop {
            sleep(POLL_INTERVAL).await;

            let wf_api: Api<Workflow> = self.management_api(CLUSTER_API_NAMESPACE);
            let mgmt_pod_api: Api<Pod> = self.management_api(CLUSTER_API_NAMESPACE);

            let result = async {
                let workload_pod_api: Api<Pod> = self.workload_api(&workload_ns).await?;
                poll_workflow_and_pods(
                    &wf_api,
                    &wf_params,
                    &mgmt_pod_api,
                    &wf_pod_params,
                    &workload_pod_api,
                    &workload_pod_params,
                )
                .await
            }
            .await;

            match classify_poll_result(result, &mut first_failure) {
                PollDecision::Terminal(outcome) => return outcome,
                PollDecision::KeepWaiting => {}
                PollDecision::RetryAfterRefresh => {
                    let _ = self.refresh_workload_now().await;
                }
            }
        }
    }
}

fn check_pod_for_unrunnable(pod: &Pod) -> Option<WatchOutcome> {
    let pod_status = pod.status.as_ref()?;
    let containers = pod_status.container_statuses.as_deref().unwrap_or_default();
    let init_containers = pod_status
        .init_container_statuses
        .as_deref()
        .unwrap_or_default();

    for status in containers.iter().chain(init_containers) {
        if let Some(reason) = status
            .state
            .as_ref()
            .and_then(|s| s.waiting.as_ref())
            .and_then(|w| w.reason.as_deref())
            && UNRUNNABLE_REASONS.contains(&reason)
        {
            return Some(WatchOutcome::ContainerUnrunnable(reason.to_owned()));
        }
    }

    None
}

/// Detect a `scenario-runner` container that was killed by the OOM killer. Unlike the Waiting
/// reasons in `check_pod_for_unrunnable`, this container *did* terminate with a real exit code —
/// but its sibling `output-collector` sidecar waits on a sentinel file that a SIGKILLed process
/// never gets to write, so the pod hangs indefinitely without this check.
///
/// Scoped to the named `scenario-runner` container specifically: this must only ever run against
/// the scenario Job's pod, never against environment pods, where an OOMKilled container is
/// expected to self-heal via the Deployment's normal restart behaviour.
fn check_scenario_runner_oomkilled(pod: &Pod) -> Option<WatchOutcome> {
    let statuses = pod.status.as_ref()?.container_statuses.as_deref()?;
    let status = statuses
        .iter()
        .find(|s| s.name == SCENARIO_RUNNER_CONTAINER)?;
    let terminated = status.state.as_ref()?.terminated.as_ref()?;

    if terminated.reason.as_deref() == Some("OOMKilled") {
        Some(WatchOutcome::ContainerUnrunnable("OOMKilled".to_owned()))
    } else {
        None
    }
}

fn check_workflow_status(wf: &Workflow) -> Option<WatchOutcome> {
    let status = wf.status.as_ref()?;
    match status.phase.as_deref() {
        Some("Succeeded") => Some(WatchOutcome::Succeeded),

        Some("Failed") | Some("Error") => {
            let msg = status
                .message
                .clone()
                .unwrap_or_else(|| "argo workflow failed".into());

            Some(WatchOutcome::Failed(msg))
        }

        Some("Running") => None,

        _ => {
            error!(?status, "unexpected workflow status");
            None
        }
    }
}

fn check_job_status(job: &Job) -> Option<WatchOutcome> {
    let status = job.status.as_ref()?;
    let conditions = status.conditions.as_deref().unwrap_or_default();
    if conditions
        .iter()
        .any(|c| c.type_ == "Complete" && c.status == "True")
    {
        return Some(WatchOutcome::Succeeded);
    }

    if conditions
        .iter()
        .any(|c| c.type_ == "Failed" && c.status == "True")
    {
        return Some(WatchOutcome::Failed("job failed".into()));
    }

    None
}

/// List `api` and apply `check` to the first matching item, if any.
async fn poll_first<K>(
    api: &Api<K>,
    params: &ListParams,
    check: impl Fn(&K) -> Option<WatchOutcome>,
) -> Result<Option<WatchOutcome>>
where
    K: Clone + std::fmt::Debug + serde::de::DeserializeOwned,
{
    let list = api.list(params).await?;

    Ok(list.items.first().and_then(check))
}

/// List pods matching `params` and check each for a permanently-stuck waiting state.
async fn poll_pods(api: &Api<Pod>, params: &ListParams) -> Result<Option<WatchOutcome>> {
    let pods = api.list(params).await?;

    Ok(pods.items.iter().find_map(check_pod_for_unrunnable))
}

/// List pods matching `params` and check each for a permanently-stuck waiting state or an
/// OOMKilled `scenario-runner` container. Only used for the scenario Job's own pod — see
/// `check_scenario_runner_oomkilled`.
async fn poll_scenario_pods(api: &Api<Pod>, params: &ListParams) -> Result<Option<WatchOutcome>> {
    let pods = api.list(params).await?;

    Ok(pods.items.iter().find_map(|pod| {
        check_pod_for_unrunnable(pod).or_else(|| check_scenario_runner_oomkilled(pod))
    }))
}

async fn poll_job_and_pods(
    job_api: &Api<Job>,
    pod_api: &Api<Pod>,
    params: &ListParams,
) -> Result<Option<WatchOutcome>> {
    if let Some(outcome) = poll_first(job_api, params, check_job_status).await? {
        return Ok(Some(outcome));
    }

    poll_scenario_pods(pod_api, params).await
}

async fn poll_workflow_and_pods(
    wf_api: &Api<Workflow>,
    wf_params: &ListParams,
    mgmt_pod_api: &Api<Pod>,
    mgmt_pod_params: &ListParams,
    workload_pod_api: &Api<Pod>,
    workload_pod_params: &ListParams,
) -> Result<Option<WatchOutcome>> {
    if let Some(outcome) = poll_first(wf_api, wf_params, check_workflow_status).await? {
        return Ok(Some(outcome));
    }

    if let Some(outcome) = poll_pods(mgmt_pod_api, mgmt_pod_params).await? {
        return Ok(Some(outcome));
    }

    poll_pods(workload_pod_api, workload_pod_params).await
}

/// What a `wait_for_job`/`wait_for_workflow` loop should do next after one poll attempt.
enum PollDecision {
    /// The poll succeeded but no terminal state was reached yet; sleep and poll again.
    KeepWaiting,
    /// A retryable error occurred and the retry window has not been exceeded; rebuild the
    /// workload client before polling again, on the chance the error was caused by an expired
    /// token.
    RetryAfterRefresh,
    /// The wait loop is done: either a terminal state was reached, a non-retryable error
    /// occurred, or the retry window was exceeded.
    Terminal(WatchOutcome),
}

/// Shared bookkeeping between `wait_for_job` and `wait_for_workflow`: tracks how long a run of
/// retryable errors has been ongoing via `first_failure`, resetting it on any successful poll,
/// and gives up once [RETRY_WINDOW] is exceeded.
fn classify_poll_result(
    result: Result<Option<WatchOutcome>>,
    first_failure: &mut Option<DateTime<Utc>>,
) -> PollDecision {
    match result {
        Ok(Some(outcome)) => PollDecision::Terminal(outcome),
        // Client api is working so clear first failure
        Ok(None) => {
            *first_failure = None;
            PollDecision::KeepWaiting
        }
        Err(e) if e.is_retryable() => {
            let first_failure_at = *first_failure.get_or_insert_with(Utc::now);
            if Utc::now() - first_failure_at > RETRY_WINDOW {
                PollDecision::Terminal(WatchOutcome::WatchErrors(
                    Error::MaxRetriesExceeded.to_string(),
                ))
            } else {
                PollDecision::RetryAfterRefresh
            }
        }
        Err(e) => PollDecision::Terminal(WatchOutcome::WatchErrors(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::workflow::WorkflowStatus;
    use k8s_openapi::api::batch::v1::{JobCondition, JobStatus};
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateTerminated, ContainerStateWaiting, ContainerStatus, PodStatus,
    };
    use simple_test_case::test_case;

    fn job_with_condition(type_: &str, status: &str) -> Job {
        Job {
            status: Some(JobStatus {
                conditions: Some(vec![JobCondition {
                    type_: type_.to_owned(),
                    status: status.to_owned(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn pod_with_waiting_reason(reason: &str) -> Pod {
        Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some(reason.to_owned()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn pod_with_terminated_container(name: &str, reason: &str, exit_code: i32) -> Pod {
        Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: name.to_owned(),
                    state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            reason: Some(reason.to_owned()),
                            exit_code,
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn retryable_error() -> Error {
        Error::Kube(kube::Error::Service(Box::new(std::io::Error::other(
            "connection reset",
        ))))
    }

    #[test]
    fn check_job_status_reports_succeeded_on_complete_condition() {
        let job = job_with_condition("Complete", "True");
        assert!(matches!(
            check_job_status(&job),
            Some(WatchOutcome::Succeeded)
        ));
    }

    #[test]
    fn check_job_status_reports_failed_on_failed_condition() {
        let job = job_with_condition("Failed", "True");
        assert!(matches!(
            check_job_status(&job),
            Some(WatchOutcome::Failed(_))
        ));
    }

    #[test]
    fn check_job_status_returns_none_while_still_running() {
        let job = job_with_condition("Complete", "False");
        assert!(check_job_status(&job).is_none());
    }

    #[test]
    fn check_job_status_returns_none_with_no_status_yet() {
        assert!(check_job_status(&Job::default()).is_none());
    }

    fn workflow_with_phase(phase: &str) -> Workflow {
        Workflow {
            status: Some(WorkflowStatus {
                phase: Some(phase.to_owned()),
                message: None,
            }),
            ..Default::default()
        }
    }

    #[test_case("Succeeded", true; "succeeded phase reports success")]
    #[test_case("Running", false; "running phase reports nothing yet")]
    #[test]
    fn check_workflow_status_handles_terminal_and_running_phases(phase: &str, is_succeeded: bool) {
        let outcome = check_workflow_status(&workflow_with_phase(phase));
        assert_eq!(
            matches!(outcome, Some(WatchOutcome::Succeeded)),
            is_succeeded
        );
    }

    #[test]
    fn check_workflow_status_reports_failed_with_message() {
        let wf = workflow_with_phase("Failed");
        assert!(matches!(
            check_workflow_status(&wf),
            Some(WatchOutcome::Failed(_))
        ));
    }

    #[test]
    fn check_pod_for_unrunnable_flags_known_unrunnable_reasons() {
        let pod = pod_with_waiting_reason("ImagePullBackOff");
        assert!(matches!(
            check_pod_for_unrunnable(&pod),
            Some(WatchOutcome::ContainerUnrunnable(_))
        ));
    }

    #[test]
    fn check_pod_for_unrunnable_ignores_unrelated_waiting_reasons() {
        let pod = pod_with_waiting_reason("ContainerCreating");
        assert!(check_pod_for_unrunnable(&pod).is_none());
    }

    #[test]
    fn check_scenario_runner_oomkilled_flags_oomkilled_scenario_runner() {
        let pod = pod_with_terminated_container(SCENARIO_RUNNER_CONTAINER, "OOMKilled", 137);
        assert!(matches!(
            check_scenario_runner_oomkilled(&pod),
            Some(WatchOutcome::ContainerUnrunnable(_))
        ));
    }

    #[test]
    fn check_scenario_runner_oomkilled_ignores_other_terminated_reasons() {
        let pod = pod_with_terminated_container(SCENARIO_RUNNER_CONTAINER, "Error", 1);
        assert!(check_scenario_runner_oomkilled(&pod).is_none());
    }

    #[test]
    fn check_scenario_runner_oomkilled_ignores_oomkilled_sidecar() {
        // The output-collector sidecar is never the scenario itself; an OOMKill there is not a
        // scenario failure.
        let pod = pod_with_terminated_container(OUTPUT_COLLECTOR, "OOMKilled", 137);
        assert!(check_scenario_runner_oomkilled(&pod).is_none());
    }

    #[test]
    fn poll_pods_used_for_environment_pods_ignores_scenario_oomkill() {
        // Environment pods must never be flagged for a scenario-runner OOMKill - poll_pods (used
        // by poll_workflow_and_pods) intentionally has no OOMKilled check at all.
        let pod = pod_with_terminated_container(SCENARIO_RUNNER_CONTAINER, "OOMKilled", 137);
        assert!(check_pod_for_unrunnable(&pod).is_none());
    }

    #[test]
    fn classify_poll_result_returns_terminal_on_success_outcome() {
        let mut first_failure = None;
        let decision = classify_poll_result(Ok(Some(WatchOutcome::Succeeded)), &mut first_failure);

        assert!(matches!(
            decision,
            PollDecision::Terminal(WatchOutcome::Succeeded)
        ));
    }

    #[test]
    fn classify_poll_result_clears_first_failure_and_keeps_waiting_on_ok_none() {
        let mut first_failure = Some(Utc::now() - Duration::seconds(60));

        let decision = classify_poll_result(Ok(None), &mut first_failure);

        assert!(matches!(decision, PollDecision::KeepWaiting));
        assert!(first_failure.is_none());
    }

    #[test]
    fn classify_poll_result_returns_terminal_on_non_retryable_error_without_starting_window() {
        let mut first_failure = None;

        let decision = classify_poll_result(
            Err(Error::Kube(kube::Error::TlsRequired)),
            &mut first_failure,
        );

        assert!(matches!(
            decision,
            PollDecision::Terminal(WatchOutcome::WatchErrors(_))
        ));
        assert!(first_failure.is_none());
    }

    #[test]
    fn classify_poll_result_retries_within_window() {
        let mut first_failure = None;

        let decision = classify_poll_result(Err(retryable_error()), &mut first_failure);

        assert!(matches!(decision, PollDecision::RetryAfterRefresh));
        assert!(first_failure.is_some());
    }

    #[test]
    fn classify_poll_result_does_not_give_up_before_retry_window_exceeded() {
        // First failure started well under the retry window ago; still within budget.
        let mut first_failure = Some(Utc::now() - RETRY_WINDOW + Duration::seconds(5));

        let decision = classify_poll_result(Err(retryable_error()), &mut first_failure);

        assert!(matches!(decision, PollDecision::RetryAfterRefresh));
    }

    /// Regression test: `first_failure` used to be set with `Option::insert`, which
    /// unconditionally overwrites the value on *every* retryable error instead of only the
    /// first. That reset the retry window's start time on every poll, so the elapsed time
    /// compared against [RETRY_WINDOW] was always ~zero and this case could never trigger.
    #[test]
    fn classify_poll_result_gives_up_once_retry_window_exceeded() {
        // First failure started well over the retry window ago.
        let mut first_failure = Some(Utc::now() - RETRY_WINDOW - Duration::seconds(5));

        let decision = classify_poll_result(Err(retryable_error()), &mut first_failure);

        assert!(matches!(
            decision,
            PollDecision::Terminal(WatchOutcome::WatchErrors(_))
        ));
    }

    #[test]
    fn classify_poll_result_recovery_resets_the_retry_window() {
        // Started failing well past the retry window already, but a successful poll should
        // reset the clock rather than carry the stale start time forward.
        let mut first_failure = Some(Utc::now() - RETRY_WINDOW - Duration::seconds(30));
        classify_poll_result(Ok(None), &mut first_failure);
        assert!(first_failure.is_none());

        // Failing again immediately after recovery should not immediately give up.
        let decision = classify_poll_result(Err(retryable_error()), &mut first_failure);

        assert!(matches!(decision, PollDecision::RetryAfterRefresh));
    }
}
