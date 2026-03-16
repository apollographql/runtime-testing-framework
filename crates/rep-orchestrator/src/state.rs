use rtf_config::formats::RepPayload;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use uuid::Uuid;

#[derive(Debug)]
pub struct TestPlanWithId {
    pub id: Uuid,
    pub payload: RepPayload,
}

#[derive(Debug, Clone)]
pub struct ServerState {
    tx_resolve: UnboundedSender<TestPlanWithId>,
}

impl ServerState {
    pub fn new() -> (Self, UnboundedReceiver<TestPlanWithId>) {
        let (tx_resolve, rx_resolve) = unbounded_channel();

        (Self { tx_resolve }, rx_resolve)
    }

    pub fn submit_test_plan(&self, id: Uuid, payload: RepPayload) -> Result<(), Box<RepPayload>> {
        self.tx_resolve
            .send(TestPlanWithId { id, payload })
            .map_err(|e| Box::new(e.0.payload))
    }
}
