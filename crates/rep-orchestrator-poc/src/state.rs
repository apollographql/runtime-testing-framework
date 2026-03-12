use crate::{rep_test_plan::RepTestPlan, resolver::TestPlanWithId};
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

    pub fn submit_test_plan(&self, id: Uuid, rtp: RepTestPlan) -> Result<(), Box<RepTestPlan>> {
        self.tx_resolve
            .send(TestPlanWithId { id, rtp })
            .map_err(|e| Box::new(e.0.rtp))
    }
}
