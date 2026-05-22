use crate::k8s::{
    FullClient, ManagementClient, Result, WatchOutcome, Workflow, WorkflowSpec, WorkloadClient,
};
use k8s_openapi::api::batch::v1::{Job, JobSpec};
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
    pub create_workflow: Resp<Result<Workflow>>,
    pub create_job: Resp<Result<Job>>,
    pub wait_for_workflow: Resp<WatchOutcome>,
    pub wait_for_job: Resp<WatchOutcome>,
    pub delete_workload_namespace: Resp<Result<()>>,
}

impl MockClient {
    /// Construct a [MockClient] with all responses set to happy path default values.
    ///
    /// Use [MockClient::default] to default all responses to unset.
    pub fn default_ok() -> Self {
        Self {
            create_workflow: Resp::new(Ok(Default::default())),
            create_job: Resp::new(Ok(Default::default())),
            wait_for_workflow: Resp::new(WatchOutcome::Succeeded),
            wait_for_job: Resp::new(WatchOutcome::Succeeded),
            delete_workload_namespace: Resp::new(Ok(())),
        }
    }
}

impl ManagementClient for MockClient {
    async fn create_argo_workflow(
        &self,
        _execution_id: &Uuid,
        _spec: WorkflowSpec,
    ) -> Result<Workflow> {
        self.create_workflow
            .take()
            .expect("create_argo_workflow called but no result configured")
    }
}

impl WorkloadClient for MockClient {
    async fn create_job(
        &self,
        _ns: &str,
        _name: &str,
        _execution_id: &Uuid,
        _spec: JobSpec,
    ) -> Result<Job> {
        self.create_job
            .take()
            .expect("create_job called but no outcome configured")
    }

    async fn wait_for_job(&self, _ns: &str, _execution_id: &Uuid) -> WatchOutcome {
        self.wait_for_job
            .take()
            .expect("wait_for_job called but no outcome configured")
    }

    async fn delete_workload_namespace(&self, _ns: &str) -> Result<()> {
        self.delete_workload_namespace
            .take()
            .expect("delete_workload_namespace called but no outcome configured")
    }
}

impl FullClient for MockClient {
    async fn wait_for_workflow(&self, _execution_id: &Uuid) -> WatchOutcome {
        self.wait_for_workflow
            .take()
            .expect("wait_for_workflow called but no outcome configured")
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
