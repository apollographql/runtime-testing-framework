use crate::db::TestRun;
use rtf_config::formats::RepPayload;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Debug)]
pub struct TestRunWithPayload {
    pub test_run: TestRun,
    pub payload: RepPayload,
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
        payload: RepPayload,
    ) -> Result<(), Box<RepPayload>> {
        self.tx_resolve
            .send(TestRunWithPayload { test_run, payload })
            .map_err(|e| Box::new(e.0.payload))
    }
}
