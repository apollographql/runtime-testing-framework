use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::ConfigMap,
};
use kube::{
    Client, Config, Resource,
    api::{Api, ObjectMeta},
    config::{KubeConfigOptions, Kubeconfig},
    core::NamespaceResourceScope,
};
use kube_runtime::{WatchStreamExt, watcher};
use std::{collections::BTreeMap, path::Path, pin::pin};
use tokio_stream::StreamExt;
use tracing::error;
use uuid::Uuid;

mod job;
mod workflow;

pub use job::{CONFIG_MAP_NAME_SCENARIO, scenario_job};
use workflow::Workflow;
pub use workflow::{WorkflowSpec, env_configmap_name, workflow_name};

pub const CLUSTER_API_NAMESPACE: &str = "cluster-api";
pub const ENVIRONMENT_CONFIG_FILENAME: &str = "environment.yaml";
pub const EXECUTION_ID_LABEL: &str = "rtf.io/execution-id";
pub const SCENARIO_CONFIG_FILENAME: &str = "scenario.yaml";
pub const TOOLBOX_IMAGE: &str =
    "us-central1-docker.pkg.dev/platform-cross-environment/platform-docker/rtf-toolbox:edge";

#[derive(Debug, Clone, Copy)]
pub enum Cluster {
    Management,
    Workload,
}

#[derive(Debug, Clone)]
pub enum WatchOutcome {
    Succeeded,
    Failed(String),
    WatcherError(String),
    StreamClosed,
}

#[derive(Clone)]
pub struct ClusterClients {
    /// rtf-mgmt — where the Argo workflows run
    management: Client,
    /// rtf-workload — where scenarios run
    workload: Client,
}

impl ClusterClients {
    /// Construct a new pair of k8s clients using the provided kubeconfig path and contexts.
    pub async fn try_new(
        path: &Path,
        management_context: &str,
        workload_context: &str,
    ) -> anyhow::Result<Self> {
        let kfg = Kubeconfig::read_from(path)?;

        Ok(Self {
            management: client_for_context(kfg.clone(), management_context).await?,
            workload: client_for_context(kfg, workload_context).await?,
        })
    }

    /// Helper for obtaining an [Api] client associated with the appropriate cluster namespace.
    fn namespaced_api<K>(&self, cluster: Cluster, ns: &str) -> Api<K>
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

    /// Create a new config map in the target cluster namespace.
    pub async fn create_configmap(
        &self,
        cluster: Cluster,
        namespace: impl Into<String>,
        configmap_name: impl Into<String>,
        file_name: &'static str,
        content: String,
    ) -> anyhow::Result<ConfigMap> {
        let ns = namespace.into();
        let cm = self
            .namespaced_api(cluster, &ns)
            .create(
                &Default::default(),
                &ConfigMap {
                    metadata: ObjectMeta {
                        name: Some(configmap_name.into()),
                        namespace: Some(ns),
                        ..Default::default()
                    },
                    data: Some(BTreeMap::from([(file_name.into(), content)])),
                    ..Default::default()
                },
            )
            .await?;

        Ok(cm)
    }

    // FIXME: we need a unique identifier for each workflow. We have the execution ID available but
    // its a UUID so that's not going to be particularly readable? But it may be the way to go...

    pub async fn create_argo_workflow(&self, execution_id: &Uuid) -> anyhow::Result<Workflow> {
        let wf = self
            .namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE)
            .create(
                &Default::default(),
                &Workflow {
                    metadata: ObjectMeta {
                        name: Some(workflow_name(execution_id)),
                        namespace: Some(CLUSTER_API_NAMESPACE.to_owned()),
                        ..Default::default()
                    },
                    spec: WorkflowSpec::for_execution_id(execution_id),
                    ..Default::default()
                },
            )
            .await?;

        Ok(wf)
    }

    pub async fn create_job(
        &self,
        cluster: Cluster,
        ns: &str,
        name: &str,
        spec: JobSpec,
    ) -> anyhow::Result<Job> {
        let job = self
            .namespaced_api(cluster, ns)
            .create(
                &Default::default(),
                &Job {
                    metadata: ObjectMeta {
                        name: Some(name.to_owned()),
                        namespace: Some(ns.to_owned()),
                        ..Default::default()
                    },
                    spec: Some(spec),
                    ..Default::default()
                },
            )
            .await?;

        Ok(job)
    }

    // FIXME: watch based on lables rather than names

    pub async fn wait_for_workflow(&self, execution_id: &Uuid) -> WatchOutcome {
        let api: Api<Workflow> = self.namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE);
        let config = watcher::Config::default()
            .fields(&format!("metadata.name={}", workflow_name(execution_id)));
        let mut stream = pin!(watcher(api, config).applied_objects());

        while let Some(res) = stream.next().await {
            let wf = match res {
                Ok(wf) => wf,
                Err(e) => return WatchOutcome::WatcherError(e.to_string()),
            };

            if let Some(status) = &wf.status {
                match status.phase.as_deref() {
                    Some("Succeeded") => return WatchOutcome::Succeeded,

                    Some("Failed") | Some("Error") => {
                        let msg = status.message.clone().unwrap_or_default();
                        return WatchOutcome::Failed(msg);
                    }

                    Some("Running") => (),

                    _ => {
                        error!(?status, "unexpected status");
                        continue;
                    }
                }
            }
        }

        WatchOutcome::StreamClosed
    }

    pub async fn wait_for_job(&self, cluster: Cluster, ns: &str, name: &str) -> WatchOutcome {
        let api: Api<Job> = self.namespaced_api(cluster, ns);
        let config = watcher::Config::default().fields(&format!("metadata.name={name}"));
        let mut stream = pin!(watcher(api, config).applied_objects());

        while let Some(res) = stream.next().await {
            let job = match res {
                Ok(job) => job,
                Err(e) => return WatchOutcome::WatcherError(e.to_string()),
            };

            if let Some(status) = &job.status {
                let conditions = status.conditions.as_deref().unwrap_or_default();
                if conditions
                    .iter()
                    .any(|c| c.type_ == "Complete" && c.status == "True")
                {
                    return WatchOutcome::Succeeded;
                }
                if conditions
                    .iter()
                    .any(|c| c.type_ == "Failed" && c.status == "True")
                {
                    return WatchOutcome::Failed("".into());
                }
            }
        }

        WatchOutcome::StreamClosed
    }
}

async fn client_for_context(kfg: Kubeconfig, context: &str) -> anyhow::Result<Client> {
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
