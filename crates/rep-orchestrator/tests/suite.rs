mod common;

use anyhow::Context;
use common::TestHelper;
use rep_orchestrator_shared::{status::Status::*, summary::TestRunSummary};
use reqwest::StatusCode;
use std::time::Duration;

#[tokio::test]
async fn full_test_run_happy_path_completes_successfully() {
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
    t.poll_for_status(ex_id, Successful, Duration::from_secs(300))
        .await;

    let resp = t
        .get(format!("test-execution/{ex_id}/log.txt"))
        .await
        .context("failed to fetch log.txt")
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "log.txt");

    // TODO: also pull the output.zip (which we will need to extract and verify)
}
