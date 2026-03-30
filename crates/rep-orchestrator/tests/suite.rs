mod common;

use common::TestHelper;
use rep_orchestrator_shared::{status::Status::*, summary::TestRunSummary};
use std::time::Duration;

#[tokio::test]
async fn trigger_reaches_provisioning_status() {
    let t = TestHelper::new();

    let run: TestRunSummary = t
        .json_post(
            "test-run/trigger",
            t.prepare_rep_payload("resources/test-plans/valid/minimal")
                .await
                .unwrap(),
        )
        .await
        .unwrap();

    let ex_id = t
        .poll_for_execution_id(run.id, Duration::from_secs(5))
        .await;
    t.poll_for_status(ex_id, Provisioning, Duration::from_secs(30))
        .await;
}
