use crate::resolver::TestPlanWithId;
use rtf_config::formats::TestPlanConfig;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ServerState {
    tx_resolve: UnboundedSender<TestPlanWithId>,
}

impl ServerState {
    pub fn new() -> (Self, UnboundedReceiver<TestPlanWithId>) {
        let (tx_resolve, rx_resolve) = unbounded_channel();

        (Self { tx_resolve }, rx_resolve)
    }

    pub fn submit_test_plan(
        &self,
        id: Uuid,
        test_plan: TestPlanConfig,
    ) -> Result<(), Box<TestPlanConfig>> {
        self.tx_resolve
            .send(TestPlanWithId { id, test_plan })
            .map_err(|e| Box::new(e.0.test_plan))
    }
}
