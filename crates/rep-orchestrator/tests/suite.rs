mod common;

use common::TestHelper;
use rep_orchestrator::{db::Status, response_types::TestRunSummary};
use reqwest::StatusCode;
use std::time::Duration;
use uuid::Uuid;

async fn trigger_rep_prepare_test_plan(t: &TestHelper) -> anyhow::Result<TestRunSummary> {
    let body = t
        .prepare_rep_payload("../rtf-cli/resources/test-plans/valid/rep-prepare")
        .await?;

    t.json_post("test-run/trigger", body).await
}

#[tokio::test]
async fn health_check_works() {
    let t = TestHelper::new();

    let resp = t.get("health").await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "{resp:?}");
}

#[tokio::test]
async fn trigger_valid_rep_test_plan_returns_200() {
    let t = TestHelper::new();

    let resp = t
        .trigger_run("../rtf-cli/resources/test-plans/valid/rep-prepare")
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "{resp:?}");
}

#[tokio::test]
async fn trigger_response_summary_is_initialising() {
    let t = TestHelper::new();

    let trs = trigger_rep_prepare_test_plan(&t).await.unwrap();

    assert_eq!(trs.current_status, Status::Initialising, "{trs:?}");
}

#[tokio::test]
async fn test_run_status_returns_200_for_known_run() {
    let t = TestHelper::new();

    let from_trigger = trigger_rep_prepare_test_plan(&t).await.unwrap();
    let resp = t
        .get(format!("test-run/{}/status", from_trigger.id))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_run_status_returns_404_for_unknown_run() {
    let t = TestHelper::new();

    let resp = t
        .get(format!("test-run/{}/status", Uuid::new_v4()))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_execution_status_returns_200_for_known_execution() {
    let t = TestHelper::new();

    let from_trigger = trigger_rep_prepare_test_plan(&t).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await; // ensure that we get the status update
    let queried: TestRunSummary = t
        .json_get(format!("test-run/{}/status", from_trigger.id))
        .await
        .unwrap();

    assert_eq!(queried.executions.len(), 1, "{:?}", queried.executions);

    let ex_id = queried.executions[0].id;
    let resp = t
        .get(format!("test-execution/{}/status", ex_id))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_execution_status_returns_404_for_unknown_execution() {
    let t = TestHelper::new();

    let resp = t
        .get(format!("test-execution/{}/status", Uuid::new_v4()))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
