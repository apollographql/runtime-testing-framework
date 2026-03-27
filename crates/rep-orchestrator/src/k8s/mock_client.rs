use crate::k8s::{Client, Cluster, Result, WatchOutcome, Workflow, WorkflowSpec};
use k8s_openapi::api::{
    batch::v1::{Job, JobSpec},
    core::v1::ConfigMap,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// A simple mock client that can hold a single optional response for each of the methods in the
/// [Client] trait.
///
/// # Assumptions
/// We only need to handle a single call to each of the methods as each event handler calls any
/// given method at most once.
#[derive(Default, Debug, Clone)]
pub struct MockClient {
    pub create_configmap: Resp<Result<ConfigMap>>,
    pub create_workflow: Resp<Result<Workflow>>,
    pub wait_for_workflow: Resp<WatchOutcome>,
}

impl MockClient {
    /// Construct a [MockClient] with all responses set to happy path default values.
    ///
    /// Use [MockClient::default] to default all responses to unset.
    pub fn default_ok() -> Self {
        Self {
            create_configmap: Resp::new(Ok(Default::default())),
            create_workflow: Resp::new(Ok(Default::default())),
            wait_for_workflow: Resp::new(WatchOutcome::Succeeded),
        }
    }
}

impl Client for MockClient {
    async fn create_configmap(
        &self,
        _cluster: Cluster,
        _namespace: &str,
        _configmap_name: &str,
        _file_name: &str,
        _content: String,
    ) -> Result<ConfigMap> {
        self.create_configmap
            .take()
            .expect("create_configmap called but no result configured")
    }

    async fn create_argo_workflow(&self, _name: &str, _spec: WorkflowSpec) -> Result<Workflow> {
        self.create_workflow
            .take()
            .expect("create_argo_workflow called but no result configured")
    }

    async fn wait_for_workflow(&self, _execution_id: &Uuid) -> WatchOutcome {
        self.wait_for_workflow
            .take()
            .expect("wait_for_workflow called but no outcome configured")
    }

    async fn create_job(&self, _ns: &str, _name: &str, _spec: JobSpec) -> Result<Job> {
        unimplemented!("not yet used in tests")
    }

    async fn wait_for_job(&self, _ns: &str, _execution_id: &Uuid) -> WatchOutcome {
        unimplemented!("not yet used in tests")
    }

    async fn delete_workload_namespace(&self, _ns: &str) -> Result<()> {
        unimplemented!("not yet used in tests")
    }
}

/// A stubbed response to a method call on [MockClient].
#[derive(Debug)]
pub struct Resp<T> {
    inner: Arc<Mutex<Option<T>>>,
}

impl<T> Resp<T> {
    pub fn new(t: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(t))),
        }
    }

    fn take(&self) -> Option<T> {
        self.inner.lock().unwrap().take()
    }
}

impl<T> Default for Resp<T> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }
}

impl<T> Clone for Resp<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
