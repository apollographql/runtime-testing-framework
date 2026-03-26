use super::{Cluster, Error, Result, WatchOutcome, Workflow, WorkflowSpec};
use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::ConfigMap,
};
use kube::config::KubeconfigError;
use std::sync::Mutex;
use uuid::Uuid;

pub struct MockClient {
    pub create_configmap_result: Mutex<Option<Result<ConfigMap>>>,
}

impl MockClient {
    pub fn ok() -> Self {
        Self {
            create_configmap_result: Mutex::new(Some(Ok(Default::default()))),
        }
    }

    pub fn configmap_err() -> Self {
        Self {
            create_configmap_result: Mutex::new(Some(Err(Error::KubeConfig(
                KubeconfigError::CurrentContextNotSet,
            )))),
        }
    }
}

impl super::Client for MockClient {
    async fn create_configmap(
        &self,
        _cluster: Cluster,
        _namespace: &str,
        _configmap_name: &str,
        _file_name: &str,
        _content: String,
    ) -> Result<ConfigMap> {
        self.create_configmap_result
            .lock()
            .unwrap()
            .take()
            .expect("create_configmap called more than once")
    }

    async fn create_argo_workflow(&self, _name: &str, _spec: WorkflowSpec) -> Result<Workflow> {
        unimplemented!("not yet used in tests")
    }

    async fn create_job(&self, _ns: &str, _name: &str, _spec: JobSpec) -> Result<Job> {
        unimplemented!("not yet used in tests")
    }

    async fn wait_for_workflow(&self, _execution_id: &Uuid) -> WatchOutcome {
        unimplemented!("not yet used in tests")
    }

    async fn wait_for_job(&self, _ns: &str, _execution_id: &Uuid) -> WatchOutcome {
        unimplemented!("not yet used in tests")
    }

    async fn delete_workload_namespace(&self, _ns: &str) -> Result<()> {
        unimplemented!("not yet used in tests")
    }
}
