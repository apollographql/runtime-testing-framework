use crate::{db::TestRun, event_loop::EventQueueState, gcs::GCSClient};
use rep_orchestrator_shared::payload::TriggerPayload;
use std::sync::Arc;

#[derive(Debug)]
pub struct TestRunWithPayload {
    pub test_run: TestRun,
    pub payload: TriggerPayload,
}

#[derive(Debug, Clone)]
pub struct ServerState {
    pub eq_state: EventQueueState,
    pub gcs_client: Arc<GCSClient>,
}

impl ServerState {
    pub fn new(eq_state: EventQueueState, gcs_client: GCSClient) -> Self {
        Self {
            eq_state,
            gcs_client: Arc::new(gcs_client),
        }
    }
}
