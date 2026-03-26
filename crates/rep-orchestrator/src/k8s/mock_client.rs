use super::{Cluster, Error, Result, WatchOutcome, Workflow, WorkflowSpec};
use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::ConfigMap,
};
use kube::config::KubeconfigError;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct MockClient {
    pub create_configmap_result: Arc<Mutex<Option<Result<ConfigMap>>>>,
    pub create_workflow_result: Arc<Mutex<Option<Result<Workflow>>>>,
    pub wait_for_workflow_outcome: Arc<Mutex<Option<WatchOutcome>>>,
    pub delete_configmap_result: Arc<Mutex<Option<Result<()>>>>,
}

impl MockClient {
    /// All configmap and workflow operations succeed. Use for `try_run` tests — the spawned
    /// `wait_and_update` task will fail to acquire a DB connection in unit tests, but that is
    /// silent and does not affect the test's assertions on the sync-path status updates.
    pub fn ok() -> Self {
        Self {
            create_configmap_result: Arc::new(Mutex::new(Some(Ok(Default::default())))),
            create_workflow_result: Arc::new(Mutex::new(Some(Ok(Default::default())))),
            wait_for_workflow_outcome: Arc::new(Mutex::new(None)),
            delete_configmap_result: Arc::new(Mutex::new(None)),
        }
    }

    /// ConfigMap creation fails; workflow methods are unimplemented.
    pub fn configmap_err() -> Self {
        Self {
            create_configmap_result: Arc::new(Mutex::new(Some(Err(Error::KubeConfig(
                KubeconfigError::CurrentContextNotSet,
            ))))),
            create_workflow_result: Arc::new(Mutex::new(None)),
            wait_for_workflow_outcome: Arc::new(Mutex::new(None)),
            delete_configmap_result: Arc::new(Mutex::new(None)),
        }
    }

    /// ConfigMap creation succeeds, workflow creation fails.
    pub fn workflow_err() -> Self {
        Self {
            create_configmap_result: Arc::new(Mutex::new(Some(Ok(Default::default())))),
            create_workflow_result: Arc::new(Mutex::new(Some(Err(Error::KubeConfig(
                KubeconfigError::CurrentContextNotSet,
            ))))),
            wait_for_workflow_outcome: Arc::new(Mutex::new(None)),
            delete_configmap_result: Arc::new(Mutex::new(None)),
        }
    }

    /// All operations succeed; `wait_for_workflow` returns `outcome`; configmap delete succeeds.
    pub fn with_workflow(outcome: WatchOutcome) -> Self {
        Self {
            create_configmap_result: Arc::new(Mutex::new(Some(Ok(Default::default())))),
            create_workflow_result: Arc::new(Mutex::new(Some(Ok(Default::default())))),
            wait_for_workflow_outcome: Arc::new(Mutex::new(Some(outcome))),
            delete_configmap_result: Arc::new(Mutex::new(Some(Ok(())))),
        }
    }

    /// Same as `with_workflow` but configmap deletion returns an error.
    pub fn with_workflow_delete_err(outcome: WatchOutcome) -> Self {
        Self {
            create_configmap_result: Arc::new(Mutex::new(Some(Ok(Default::default())))),
            create_workflow_result: Arc::new(Mutex::new(Some(Ok(Default::default())))),
            wait_for_workflow_outcome: Arc::new(Mutex::new(Some(outcome))),
            delete_configmap_result: Arc::new(Mutex::new(Some(Err(Error::KubeConfig(
                KubeconfigError::CurrentContextNotSet,
            ))))),
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
            .expect("create_configmap called but no result configured")
    }

    async fn create_argo_workflow(&self, _name: &str, _spec: WorkflowSpec) -> Result<Workflow> {
        self.create_workflow_result
            .lock()
            .unwrap()
            .take()
            .expect("create_argo_workflow called but no result configured")
    }

    async fn create_job(&self, _ns: &str, _name: &str, _spec: JobSpec) -> Result<Job> {
        unimplemented!("not yet used in tests")
    }

    async fn wait_for_workflow(&self, _execution_id: &Uuid) -> WatchOutcome {
        self.wait_for_workflow_outcome
            .lock()
            .unwrap()
            .take()
            .expect("wait_for_workflow called but no outcome configured")
    }

    async fn wait_for_job(&self, _ns: &str, _execution_id: &Uuid) -> WatchOutcome {
        unimplemented!("not yet used in tests")
    }

    async fn delete_management_configmap(&self, _namespace: &str, _name: &str) -> Result<()> {
        self.delete_configmap_result
            .lock()
            .unwrap()
            .take()
            .expect("delete_management_configmap called but no result configured")
    }

    async fn delete_workload_namespace(&self, _ns: &str) -> Result<()> {
        unimplemented!("not yet used in tests")
    }
}
