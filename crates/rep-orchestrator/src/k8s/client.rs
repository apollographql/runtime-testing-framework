use crate::k8s::{
    CLUSTER_API_NAMESPACE, FullClient, ManagementClient, OUTPUT_COLLECTOR, Result, WatchOutcome,
    Workflow, WorkflowSpec, WorkloadClient, workflow_name,
};
use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::{Namespace, Pod, ServiceAccount},
    rbac::v1::{ClusterRole, ClusterRoleBinding, PolicyRule, Role, RoleBinding, RoleRef, Subject},
};
use kube::{
    Client, Config, Resource,
    api::{Api, ObjectMeta, Patch, PatchParams},
    config::{KubeConfigOptions, Kubeconfig},
    core::NamespaceResourceScope,
};
use kube_runtime::{WatchStreamExt, watcher};
use rep_orchestrator_shared::EXECUTION_ID_LABEL;
use std::{collections::BTreeMap, pin::pin, result};
use tokio_stream::StreamExt;
use tracing::{error, warn};
use uuid::Uuid;

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

/// Typestate marker for a [ClusterClients] slot that holds an initialised [Client].
#[derive(Clone)]
pub struct Available(Client);

/// Typestate marker for a [ClusterClients] slot that has not been populated and is therefore
/// unusable from this instance.
#[derive(Clone)]
pub struct Unavailable;

#[derive(Clone)]
pub struct ClusterClients<M, W> {
    management: M,
    workload: W,
}

impl ClusterClients<Available, Unavailable> {
    /// Construct a client that can only interact with the management cluster.
    pub async fn try_new_management() -> Result<Self> {
        let management = Client::try_from(Config::incluster_env()?)?;

        Ok(Self {
            management: Available(management),
            workload: Unavailable,
        })
    }
}

impl ClusterClients<Unavailable, Available> {
    /// Construct a client that can only interact with the workload cluster.
    pub async fn try_new_workload(workload_path: &str, workload_context: &str) -> Result<Self> {
        let kfg = Kubeconfig::read_from(workload_path)?;

        Ok(Self {
            management: Unavailable,
            workload: Available(client_for_context(kfg, workload_context).await?),
        })
    }
}

impl ClusterClients<Available, Available> {
    /// Construct a client that can interact with both clusters.
    pub async fn try_new_full(workload_path: &str, workload_context: &str) -> Result<Self> {
        let management = Client::try_from(Config::incluster_env()?)?;
        let kfg = Kubeconfig::read_from(workload_path)?;

        Ok(Self {
            management: Available(management),
            workload: Available(client_for_context(kfg, workload_context).await?),
        })
    }
}

impl<W> ClusterClients<Available, W> {
    fn management_api<K>(&self, ns: &str) -> Api<K>
    where
        K: Resource<Scope = NamespaceResourceScope>,
        <K as Resource>::DynamicType: Default,
    {
        Api::namespaced(self.management.0.clone(), ns)
    }
}

impl<M> ClusterClients<M, Available> {
    fn workload_api<K>(&self, ns: &str) -> Api<K>
    where
        K: Resource<Scope = NamespaceResourceScope>,
        <K as Resource>::DynamicType: Default,
    {
        Api::namespaced(self.workload.0.clone(), ns)
    }

    fn workload_client(&self) -> Client {
        self.workload.0.clone()
    }

    async fn create_output_collector_rbac(&self, ns: &str) -> Result<()> {
        let pp = PatchParams::apply("rep-orchestrator");

        let sa_api: Api<ServiceAccount> = self.workload_api(ns);
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

        let role_api: Api<Role> = self.workload_api(ns);
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

        let rb_api: Api<RoleBinding> = self.workload_api(ns);
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

        let cr_api: Api<ClusterRole> = Api::all(self.workload_client());
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
        let crb_api: Api<ClusterRoleBinding> = Api::all(self.workload_client());
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

impl<W: Clone + Send + Sync + 'static> ManagementClient for ClusterClients<Available, W> {
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

impl<M: Clone + Send + Sync + 'static> WorkloadClient for ClusterClients<M, Available> {
    async fn create_job(
        &self,
        ns: &str,
        name: &str,
        execution_id: &Uuid,
        spec: JobSpec,
    ) -> Result<Job> {
        self.create_output_collector_rbac(ns).await?;

        let job = self
            .workload_api(ns)
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

    async fn wait_for_job(&self, ns: &str, execution_id: &Uuid) -> WatchOutcome {
        // Watch for the job to complete
        let job_api: Api<Job> = self.workload_api(ns);
        let job_labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let job_config = watcher::Config::default().labels(&job_labels);
        let mut job_stream = pin!(watcher(job_api, job_config).applied_objects());

        // Watch the pods being deployed to the workload cluster to make sure they do not get into an unrunnable state
        let pod_api: Api<Pod> = self.workload_api(ns);
        let pod_labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let pod_config = watcher::Config::default().labels(&pod_labels);
        let mut pod_stream = pin!(watcher(pod_api, pod_config).applied_objects());

        loop {
            tokio::select! {
                Some(res) = job_stream.next() => {
                    if let Some(outcome) = handle_job_event(res) { return outcome; }
                }
                Some(res) = pod_stream.next() => {
                    if let Some(outcome) = handle_pod_watch_event(res) { return outcome; }
                }
                else => return WatchOutcome::StreamClosed
            }
        }
    }

    async fn delete_workload_namespace(&self, ns: &str) -> Result<()> {
        let crb_api: Api<ClusterRoleBinding> = Api::all(self.workload_client());
        if let Err(e) = crb_api
            .delete(&format!("{OUTPUT_COLLECTOR}-{ns}"), &Default::default())
            .await
        {
            warn!("failed to delete ClusterRoleBinding {OUTPUT_COLLECTOR}-{ns}: {e}");
        }

        let api: Api<Namespace> = Api::all(self.workload_client());
        api.delete(ns, &Default::default()).await?;

        Ok(())
    }
}

impl FullClient for ClusterClients<Available, Available> {
    async fn wait_for_workflow(&self, execution_id: &Uuid) -> WatchOutcome {
        // Watch for the workflow to complete
        let wf_api: Api<Workflow> = self.management_api(CLUSTER_API_NAMESPACE);
        let wf_labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let wf_config = watcher::Config::default().labels(&wf_labels);
        let mut wf_stream = pin!(watcher(wf_api, wf_config).applied_objects());

        // Watch the argo workflow pods to make sure they do not get into an unrunnable state
        // This is not handled particularly well by Argo natively
        let mgmt_pod_api: Api<Pod> = self.management_api(CLUSTER_API_NAMESPACE);
        let wf_pod_labels = format!("workflows.argoproj.io/workflow=provision-env-{execution_id}");
        let wf_pod_config = watcher::Config::default().labels(&wf_pod_labels);
        let mut mgmt_pod_stream = pin!(watcher(mgmt_pod_api, wf_pod_config).applied_objects());

        // Watch the pods being deployed to the workload cluster to make sure they do not get into an unrunnable state
        let workload_ns = execution_id.to_string();
        let workload_pod_api: Api<Pod> = self.workload_api(&workload_ns);
        let workload_pod_config = watcher::Config::default();
        let mut workload_pod_stream =
            pin!(watcher(workload_pod_api, workload_pod_config).applied_objects());

        loop {
            tokio::select! {
                Some(res) = wf_stream.next() => {
                    if let Some(outcome) = handle_workflow_event(res) { return outcome; }
                }
                Some(res) = mgmt_pod_stream.next() => {
                    if let Some(outcome) = handle_pod_watch_event(res) { return outcome; }
                }
                Some(res) = workload_pod_stream.next() => {
                    if let Some(outcome) = handle_pod_watch_event(res) { return outcome; }
                }
                else => return WatchOutcome::StreamClosed
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

fn handle_pod_watch_event(res: result::Result<Pod, watcher::Error>) -> Option<WatchOutcome> {
    match res {
        Ok(pod) => check_pod_for_unrunnable(&pod),
        Err(e) => Some(WatchOutcome::WatcherError(e.to_string())),
    }
}

fn handle_workflow_event(res: result::Result<Workflow, watcher::Error>) -> Option<WatchOutcome> {
    let wf = match res {
        Ok(wf) => wf,
        Err(e) => return Some(WatchOutcome::WatcherError(e.to_string())),
    };

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

fn handle_job_event(res: result::Result<Job, watcher::Error>) -> Option<WatchOutcome> {
    let job = match res {
        Ok(job) => job,
        Err(e) => return Some(WatchOutcome::WatcherError(e.to_string())),
    };

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
