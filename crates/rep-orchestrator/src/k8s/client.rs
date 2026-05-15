use crate::k8s::{
    self, CLUSTER_API_NAMESPACE, Cluster, EXECUTION_ID_LABEL, Result, WatchOutcome, Workflow,
    WorkflowSpec, workflow_name,
};
use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::{Namespace, Pod, ServiceAccount},
    rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject},
};
use kube::{
    Client, Config, Resource,
    api::{Api, ObjectMeta, Patch, PatchParams},
    config::{KubeConfigOptions, Kubeconfig},
    core::NamespaceResourceScope,
};
use kube_runtime::{WatchStreamExt, watcher};
use std::{collections::BTreeMap, pin::pin, result};
use tokio_stream::StreamExt;
use tracing::error;
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

#[derive(Clone)]
pub struct ClusterClients {
    /// rtf-mgmt — where the Argo workflows run
    management: Client,
    /// rtf-workload — where we create ephemeral namespaces for running scenarios
    workload: Client,
}

impl ClusterClients {
    /// Construct clients where the management client uses the pod's own in-cluster credentials
    /// and the workload client uses an explicit kubeconfig file. Used in prod where the
    /// orchestrator runs inside the mgmt cluster.
    pub async fn try_new(workload_path: &str, workload_context: &str) -> Result<Self> {
        let management = Client::try_from(Config::incluster_env()?)?;
        let kfg = Kubeconfig::read_from(workload_path)?;

        Ok(Self {
            management,
            workload: client_for_context(kfg, workload_context).await?,
        })
    }

    /// Helper for obtaining an [Api] client associated with the appropriate cluster namespace.
    pub fn namespaced_api<K>(&self, cluster: Cluster, ns: &str) -> Api<K>
    where
        K: Resource<Scope = NamespaceResourceScope>,
        <K as Resource>::DynamicType: Default,
    {
        let client = match cluster {
            Cluster::Management => self.management.clone(),
            Cluster::Workload => self.workload.clone(),
        };

        Api::namespaced(client, ns)
    }

    async fn create_output_collector_rbac(&self, ns: &str) -> Result<()> {
        let pp = PatchParams::apply("rep-orchestrator");

        let sa_api: Api<ServiceAccount> = self.namespaced_api(Cluster::Workload, ns);
        sa_api
            .patch(
                "output-collector",
                &pp,
                &Patch::Apply(&ServiceAccount {
                    metadata: ObjectMeta {
                        name: Some("output-collector".to_owned()),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            .await?;

        let role_api: Api<Role> = self.namespaced_api(Cluster::Workload, ns);
        role_api
            .patch(
                "output-collector",
                &pp,
                &Patch::Apply(&Role {
                    metadata: ObjectMeta {
                        name: Some("output-collector".to_owned()),
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
                    ]),
                }),
            )
            .await?;

        let rb_api: Api<RoleBinding> = self.namespaced_api(Cluster::Workload, ns);
        rb_api
            .patch(
                "output-collector",
                &pp,
                &Patch::Apply(&RoleBinding {
                    metadata: ObjectMeta {
                        name: Some("output-collector".to_owned()),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    },
                    role_ref: RoleRef {
                        api_group: "rbac.authorization.k8s.io".to_owned(),
                        kind: "Role".to_owned(),
                        name: "output-collector".to_owned(),
                    },
                    subjects: Some(vec![Subject {
                        kind: "ServiceAccount".to_owned(),
                        name: "output-collector".to_owned(),
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

impl k8s::Client for ClusterClients {
    async fn create_argo_workflow(
        &self,
        execution_id: &Uuid,
        spec: WorkflowSpec,
    ) -> Result<Workflow> {
        let wf = self
            .namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE)
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

    async fn create_job(
        &self,
        ns: &str,
        name: &str,
        execution_id: &Uuid,
        spec: JobSpec,
    ) -> Result<Job> {
        self.create_output_collector_rbac(ns).await?;

        let job = self
            .namespaced_api(Cluster::Workload, ns)
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

    async fn wait_for_workflow(&self, execution_id: &Uuid) -> WatchOutcome {
        // Watch for the workflow to complete
        let wf_api: Api<Workflow> = self.namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE);
        let wf_labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let wf_config = watcher::Config::default().labels(&wf_labels);
        let mut wf_stream = pin!(watcher(wf_api, wf_config).applied_objects());

        // Watch the argo workflow pods to make sure they do not get into an unrunnable state
        // This is not handled particularly well by Argo natively
        let mgmt_pod_api: Api<Pod> =
            self.namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE);
        let wf_pod_labels = format!("workflows.argoproj.io/workflow=provision-env-{execution_id}");
        let wf_pod_config = watcher::Config::default().labels(&wf_pod_labels);
        let mut mgmt_pod_stream = pin!(watcher(mgmt_pod_api, wf_pod_config).applied_objects());

        // Watch the pods being deployed to the workload cluster to make sure they do not get into an unrunnable state

        // TODO This should move to the orchestrator cli once we switch to using that and not the inline
        // shell logic
        let workload_ns = execution_id.to_string();
        let workload_pod_api: Api<Pod> = self.namespaced_api(Cluster::Workload, &workload_ns);
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

    async fn wait_for_job(&self, ns: &str, execution_id: &Uuid) -> WatchOutcome {
        // Watch for the job to complete
        let job_api: Api<Job> = self.namespaced_api(Cluster::Workload, ns);
        let job_labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let job_config = watcher::Config::default().labels(&job_labels);
        let mut job_stream = pin!(watcher(job_api, job_config).applied_objects());

        // Watch the pods being deployed to the workload cluster to make sure they do not get into an unrunnable state
        let pod_api: Api<Pod> = self.namespaced_api(Cluster::Workload, ns);
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
        let api: Api<Namespace> = Api::all(self.workload.clone());
        api.delete(ns, &Default::default()).await?;

        Ok(())
    }
}

fn check_pod_for_unrunnable(pod: &Pod) -> Option<WatchOutcome> {
    let statuses = pod.status.as_ref()?.container_statuses.as_deref()?;
    for status in statuses {
        if let Some(waiting) = status.state.as_ref()?.waiting.as_ref()
            && let Some(reason) = &waiting.reason
            && UNRUNNABLE_REASONS.contains(&reason.as_str())
        {
            return Some(WatchOutcome::ContainerUnrunnable(reason.clone()));
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
