use crate::{
    db::TestRun,
    event_loop::{EventQueueState, SubmitError},
};
use rep_orchestrator_shared::payload::TriggerPayload;

#[derive(Debug)]
pub struct TestRunWithPayload {
    pub test_run: TestRun,
    pub payload: TriggerPayload,
}

#[derive(Debug, Clone)]
pub struct ServerState {
    eq_state: EventQueueState,
}

impl ServerState {
    pub fn new(eq_state: EventQueueState) -> Self {
        Self { eq_state }
    }

    pub async fn try_submit_test_plan(
        &self,
        test_run: TestRun,
        payload: TriggerPayload,
    ) -> Result<(), SubmitError> {
        self.eq_state.try_submit_test_plan(test_run, payload).await
    }
}
