mod common;

use common::TestHelper;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::Api;
use rep_orchestrator::k8s::{CLUSTER_API_NAMESPACE, Cluster};
use rep_orchestrator_shared::{
    status::Status::{self, *},
    summary::TestRunSummary,
};
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
async fn health_returns_200() {
    let t = TestHelper::new();

    let resp = t.get("health").await.unwrap();

    assert_status!(resp, StatusCode::OK);
}

#[tokio::test]
async fn trigger_valid_rep_test_plan_returns_200() {
    let t = TestHelper::new();

    let resp = t
        .trigger_run("../rtf-cli/resources/test-plans/valid/rep-prepare")
        .await
        .unwrap();

    assert_status!(resp, StatusCode::OK);
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

    assert_status!(resp, StatusCode::OK);
}

#[tokio::test]
async fn run_status_returns_404_for_unknown_run() {
    let t = TestHelper::new();

    let resp = t
        .get(format!("test-run/{}/status", Uuid::new_v4()))
        .await
        .unwrap();

    assert_status!(resp, StatusCode::NOT_FOUND);
}

// Helper for writing status update tests that need a valid test run and execution to work with.
// Polls until the event loop has set Provisioning, so callers start from a known stable state.
async fn prepare_status_update_test(t: &TestHelper) -> Uuid {
    let from_trigger = trigger_rep_prepare_test_plan(t).await.unwrap();
    let ex_id = t
        .poll_for_execution_id(from_trigger.id, Duration::from_secs(5))
        .await;
    t.poll_for_status(ex_id, Provisioning, Duration::from_secs(30))
        .await;
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

    assert_status!(resp, StatusCode::OK);
}

#[tokio::test]
async fn execution_status_returns_404_for_unknown_execution() {
    let t = TestHelper::new();

    let resp = t
        .get(format!("test-execution/{}/status", Uuid::new_v4()))
        .await
        .unwrap();

    assert_status!(resp, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn trigger_creates_configmap_and_sets_provisioning() {
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

    let cm_name = format!("environment-config-{ex_id}");

    let kube_clients = t.kube_clients().await;
    let api: Api<ConfigMap> =
        kube_clients.namespaced_api(Cluster::Management, CLUSTER_API_NAMESPACE);
    api.get(&cm_name).await.unwrap_or_else(|e| {
        panic!("ConfigMap '{cm_name}' not found in cluster-api namespace: {e}")
    });
}

// FIXME: These tests are no longer going to work as originally written now that the event loop is
// updating execution statuses. Originally, the API calls being made here were the only things
// updating statuses, now we end up racing with the status updates from the orchestrator itself.
// -> We can probably rewrite them as unit tests using https://docs.rs/axum-test/latest/axum_test/

// // Helper for the parameterised test below
// fn su(status: Status, exit_code: Option<u8>) -> SetStatusPayload {
//     SetStatusPayload {
//         status,
//         message: None,
//         exit_code,
//     }
// }

// #[test_case(&[su(Running, None), su(Successful, None)]; "successful")]
// #[test_case(&[su(Running, None), su(Successful, Some(0))]; "successful with 0 exit code")]
// #[test_case(&[su(Running, None), su(Failed, Some(1))]; "failed")]
// #[test_case(&[su(Unrunnable, None)]; "unrunnable")]
// #[tokio::test]
// async fn execution_status_valid_update_sequence_accepted(payloads: &[SetStatusPayload]) {
//     let t = TestHelper::new();

//     let ex_id = prepare_status_update_test(&t).await;
//     let mut final_statuses = vec![
//         Initialising,
//         Resolving,
//         Provisioning,
//         Provisioning,
//         Provisioning,
//     ];

//     for payload in payloads.iter() {
//         let update: StatusUpdate = t
//             .json_post(format!("test-execution/{ex_id}/status"), payload)
//             .await
//             .unwrap();

//         assert_eq!(update.status, payload.status);
//         final_statuses.push(update.status);
//     }

//     let queried: TestExecutionSummary = t
//         .json_get(format!("test-execution/{ex_id}/status"))
//         .await
//         .unwrap();

//     let queried_statuses: Vec<Status> = queried.status_history.iter().map(|u| u.status).collect();
//     final_statuses.reverse(); // order in the summary is most recent first

//     assert_eq!(queried_statuses, final_statuses);
// }

// #[test_case(&[su(Running, None)], su(Provisioning, None); "status rollback")]
// #[test_case(&[su(Failed, Some(1))], su(Successful, None); "second terminal status")]
// #[test_case(&[], su(Failed, None); "failed without exit code")]
// #[test_case(&[], su(Failed, Some(0)); "failed with 0 exit code")]
// #[test_case(&[], su(Successful, Some(1)); "successful with non-0 exit code")]
// #[test_case(&[], su(Unrunnable, Some(2)); "unrunnable with exit code")]
// #[test_case(&[], su(Running, Some(3)); "non-terminal with exit code")]
// #[tokio::test]
// async fn execution_status_invalid_update_sequence_returns_400(
//     valid_payloads: &[SetStatusPayload],
//     invalid_payload: SetStatusPayload,
// ) {
//     let t = TestHelper::new();

//     let ex_id = prepare_status_update_test(&t).await;

//     for payload in valid_payloads.iter() {
//         let update: StatusUpdate = t
//             .json_post(format!("test-execution/{ex_id}/status"), payload)
//             .await
//             .unwrap();

//         assert_eq!(update.status, payload.status);
//     }

//     let resp = t
//         .post(format!("test-execution/{ex_id}/status"), invalid_payload)
//         .await
//         .unwrap();

//     assert_eq!(
//         resp.status(),
//         StatusCode::BAD_REQUEST,
//         "{:?}",
//         resp.json::<serde_json::Value>().await.unwrap()
//     );
// }

// #[tokio::test]
// async fn execution_status_update_sets_exit_code() {
//     let t = TestHelper::new();

//     let ex_id = prepare_status_update_test(&t).await;

//     let update: StatusUpdate = t
//         .json_post(
//             format!("test-execution/{ex_id}/status"),
//             su(Failed, Some(42)),
//         )
//         .await
//         .unwrap();

//     assert_eq!(update.status, Failed);

//     let summary: TestExecutionSummary = t
//         .json_get(format!("test-execution/{ex_id}/status"))
//         .await
//         .unwrap();

//     assert_eq!(summary.exit_code, Some(42));
// }
