mod common;

use common::TestHelper;
use rep_orchestrator::{
    db::{
        Status::{self, *},
        StatusUpdate,
    },
    endpoints::execution_status::SetStatusPayload,
    response_types::{TestExecutionSummary, TestRunSummary},
};
use reqwest::StatusCode;
use simple_test_case::test_case;
use std::time::Duration;
use uuid::Uuid;

async fn trigger_rep_prepare_test_plan(t: &TestHelper) -> anyhow::Result<TestRunSummary> {
    let body = t
        .prepare_rep_payload("../rtf-cli/resources/test-plans/valid/rep-prepare")
        .await?;

    t.json_post("test-run/trigger", body).await
}

#[tokio::test]
async fn health_returns_200() {
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
async fn run_status_returns_200_for_known_run() {
    let t = TestHelper::new();

    let from_trigger = trigger_rep_prepare_test_plan(&t).await.unwrap();
    let resp = t
        .get(format!("test-run/{}/status", from_trigger.id))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn run_status_returns_404_for_unknown_run() {
    let t = TestHelper::new();

    let resp = t
        .get(format!("test-run/{}/status", Uuid::new_v4()))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// Helper for writing status update tests that need a valid test run and execution to work with
async fn prepare_status_update_test(t: &TestHelper) -> Uuid {
    let from_trigger = trigger_rep_prepare_test_plan(t).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await; // ensure that we get the status update
    let queried: TestRunSummary = t
        .json_get(format!("test-run/{}/status", from_trigger.id))
        .await
        .unwrap();

    assert_eq!(queried.executions.len(), 1, "{:?}", queried.executions);

    let ex_id = queried.executions[0].id;
    let initial_status_history = &queried.executions[0].status_history;
    let initial_statuses: Vec<Status> = initial_status_history.iter().map(|u| u.status).collect();

    assert_eq!(initial_statuses, vec![Resolving, Initialising], "initial");

    ex_id
}

#[tokio::test]
async fn execution_status_returns_200_for_known_execution() {
    let t = TestHelper::new();

    let ex_id = prepare_status_update_test(&t).await;
    let resp = t
        .get(format!("test-execution/{}/status", ex_id))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn execution_status_returns_404_for_unknown_execution() {
    let t = TestHelper::new();

    let resp = t
        .get(format!("test-execution/{}/status", Uuid::new_v4()))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// Helper for the parameterised test below
fn su(status: Status, exit_code: Option<u8>) -> SetStatusPayload {
    SetStatusPayload {
        status,
        message: None,
        exit_code,
    }
}

#[test_case(&[su(Provisioning, None), su(Running, None), su(Successful, None)]; "successful")]
#[test_case(&[su(Provisioning, None), su(Running, None), su(Successful, Some(0))]; "successful with 0 exit code")]
#[test_case(&[su(Provisioning, None), su(Running, None), su(Failed, Some(1))]; "failed")]
#[test_case(&[su(Provisioning, None), su(Unrunnable, None)]; "unrunnable")]
#[tokio::test]
async fn execution_status_valid_update_sequence_accepted(payloads: &[SetStatusPayload]) {
    let t = TestHelper::new();

    let ex_id = prepare_status_update_test(&t).await;
    let mut final_statuses = vec![Initialising, Resolving];

    for payload in payloads.iter() {
        let update: StatusUpdate = t
            .json_post(format!("test-execution/{ex_id}/status"), payload)
            .await
            .unwrap();

        assert_eq!(update.status, payload.status);
        final_statuses.push(update.status);
    }

    let queried: TestExecutionSummary = t
        .json_get(format!("test-execution/{ex_id}/status"))
        .await
        .unwrap();

    let queried_statuses: Vec<Status> = queried.status_history.iter().map(|u| u.status).collect();
    final_statuses.reverse(); // order in the summary is most recent first

    assert_eq!(queried_statuses, final_statuses);
}

#[test_case(&[su(Running, None)], su(Provisioning, None); "status rollback")]
#[test_case(&[su(Failed, Some(1))], su(Successful, None); "second terminal status")]
#[test_case(&[], su(Failed, None); "failed without exit code")]
#[test_case(&[], su(Failed, Some(0)); "failed with 0 exit code")]
#[test_case(&[], su(Successful, Some(1)); "successful with non-0 exit code")]
#[test_case(&[], su(Unrunnable, Some(2)); "unrunnable with exit code")]
#[test_case(&[], su(Running, Some(3)); "non-terminal with exit code")]
#[tokio::test]
async fn execution_status_invalid_update_sequence_returns_400(
    valid_payloads: &[SetStatusPayload],
    invalid_payload: SetStatusPayload,
) {
    let t = TestHelper::new();

    let ex_id = prepare_status_update_test(&t).await;

    for payload in valid_payloads.iter() {
        let update: StatusUpdate = t
            .json_post(format!("test-execution/{ex_id}/status"), payload)
            .await
            .unwrap();

        assert_eq!(update.status, payload.status);
    }

    let resp = t
        .post(format!("test-execution/{ex_id}/status"), invalid_payload)
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "{:?}",
        resp.json::<serde_json::Value>().await.unwrap()
    );
}

#[tokio::test]
async fn execution_status_update_sets_exit_code() {
    let t = TestHelper::new();

    let ex_id = prepare_status_update_test(&t).await;

    let update: StatusUpdate = t
        .json_post(
            format!("test-execution/{ex_id}/status"),
            su(Failed, Some(42)),
        )
        .await
        .unwrap();

    assert_eq!(update.status, Failed);

    let summary: TestExecutionSummary = t
        .json_get(format!("test-execution/{ex_id}/status"))
        .await
        .unwrap();

    assert_eq!(summary.exit_code, Some(42));
}
