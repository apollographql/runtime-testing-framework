use crate::{db::TestRun, event_loop::EventQueueState};
use rep_orchestrator_shared::payload::TriggerPayload;

#[derive(Debug)]
pub struct TestRunWithPayload {
    pub test_run: TestRun,
    pub payload: TriggerPayload,
}

#[derive(Debug, Clone)]
pub struct ServerState {
    pub eq_state: EventQueueState,
}

impl ServerState {
    pub fn new(eq_state: EventQueueState) -> Self {
        Self { eq_state }
    }
}
