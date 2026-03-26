use crate::k8s::{
    self, CLUSTER_API_NAMESPACE, Cluster, EXECUTION_ID_LABEL, Result, WatchOutcome, Workflow,
    WorkflowSpec,
};
use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::{ConfigMap, Namespace},
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

#[derive(Clone)]
pub struct ClusterClients {
    /// rtf-mgmt — where the Argo workflows run
    management: Client,
    /// rtf-workload — where we create ephemeral namespaces for running scenarios
    workload: Client,
}

impl ClusterClients {
    /// Construct a new pair of k8s clients using the provided kubeconfig path and contexts.
    pub async fn try_new(
        path: &Path,
        management_context: &str,
        workload_context: &str,
    ) -> Result<Self> {
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
    async fn create_configmap(
        &self,
        cluster: Cluster,
        namespace: &str,
        configmap_name: &str,
        file_name: &str,
        content: String,
    ) -> Result<ConfigMap> {
        let cm = self
            .namespaced_api(cluster, namespace)
            .create(
                &Default::default(),
                &ConfigMap {
                    metadata: ObjectMeta {
                        name: Some(configmap_name.to_owned()),
                        namespace: Some(namespace.to_owned()),
                        ..Default::default()
                    },
                    data: Some(BTreeMap::from([(file_name.to_owned(), content)])),
                    ..Default::default()
                },
            )
            .await?;

        Ok(cm)
    }

    async fn create_argo_workflow(&self, name: &str, spec: WorkflowSpec) -> Result<Workflow> {
        let wf = self
            .namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE)
            .create(
                &Default::default(),
                &Workflow {
                    metadata: ObjectMeta {
                        name: Some(name.to_owned()),
                        namespace: Some(CLUSTER_API_NAMESPACE.to_owned()),
                        ..Default::default()
                    },
                    spec,
                    ..Default::default()
                },
            )
            .await?;

        Ok(wf)
    }

    async fn create_job(&self, ns: &str, name: &str, spec: JobSpec) -> Result<Job> {
        let job = self
            .namespaced_api(Cluster::Workload, ns)
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

    async fn wait_for_workflow(&self, execution_id: &Uuid) -> WatchOutcome {
        let api: Api<Workflow> = self.namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE);
        let labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let config = watcher::Config::default().labels(&labels);
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

    async fn wait_for_job(&self, ns: &str, execution_id: &Uuid) -> WatchOutcome {
        let api: Api<Job> = self.namespaced_api(Cluster::Workload, ns);
        let labels = format!("{EXECUTION_ID_LABEL}={execution_id}");
        let config = watcher::Config::default().labels(&labels);
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

    async fn delete_management_configmap(&self, namespace: &str, name: &str) -> Result<()> {
        let api: Api<ConfigMap> = self.namespaced_api(Cluster::Management, namespace);
        api.delete(name, &Default::default()).await?;
        Ok(())
    }

    async fn delete_workload_namespace(&self, ns: &str) -> Result<()> {
        let api: Api<Namespace> = Api::all(self.workload.clone());
        api.delete(ns, &Default::default()).await?;

        Ok(())
    }
}
