use crate::db::TestRun;
use rep_orchestrator_shared::payload::TriggerPayload;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Debug)]
pub struct TestRunWithPayload {
    pub test_run: TestRun,
    pub payload: TriggerPayload,
}

#[derive(Debug, Clone)]
pub struct ServerState {
    tx_resolve: UnboundedSender<TestRunWithPayload>,
}

impl ServerState {
    pub fn new() -> (Self, UnboundedReceiver<TestRunWithPayload>) {
        let (tx_resolve, rx_resolve) = unbounded_channel();

        (Self { tx_resolve }, rx_resolve)
    }

    pub fn submit_test_plan(
        &self,
        test_run: TestRun,
        payload: TriggerPayload,
    ) -> Result<(), Box<TriggerPayload>> {
        self.tx_resolve
            .send(TestRunWithPayload { test_run, payload })
            .map_err(|e| Box::new(e.0.payload))
    }
}
