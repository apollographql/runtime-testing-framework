mod common;

use common::TestHelper;
use rep_orchestrator::event_loop::{MSG_ARGO_WAIT, MSG_JOB_WAIT};
use rep_orchestrator_shared::{status::Status::*, summary::TestRunSummary};
use serial_test::serial;
use simple_test_case::test_case;
use std::time::Duration;

#[tokio::test]
#[serial]
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

    // Wait for the run to enter a successful status
    let ex_id = t
        .poll_for_execution_id(run.id, Duration::from_secs(5))
        .await;
    t.poll_for_status(ex_id, Successful, Duration::from_secs(180))
        .await;

    // Fetch and validate the log output
    let log_txt = t
        .get_text(format!("test-execution/{ex_id}/log.txt"))
        .await
        .unwrap();

    assert!(log_txt.ends_with("hello, world!\n"), "{log_txt:?}");

    // Fetch and validate the output.zip
    let mut zip = t.get_output_zip(ex_id).await.unwrap();

    assert_eq!(zip.rtf_output_content(), "hello, world!\n");
    assert!(zip.contains_path_prefix("output/logs/"));

    zip.read_json("output/events.json").unwrap();
    zip.read_json("output/resource-metrics.json").unwrap();

    assert_eq!(
        zip.read_prometheus_query("environment", "environment_up")
            .unwrap(),
        t.prometheus_query_with_namespace_filter(ex_id, "environment_up", "up"),
    );
    assert_eq!(
        zip.read_prometheus_query("scenario", "scenario_up")
            .unwrap(),
        t.prometheus_query_with_namespace_filter(ex_id, "scenario_up", "up"),
    );
}

#[tokio::test]
#[serial]
async fn full_test_run_happy_path_completes_successfully_with_null_environment() {
    let t = TestHelper::new();
    let run: TestRunSummary = t
        .json_post(
            "test-run/trigger",
            t.prepare_rep_payload("resources/test-plans/valid/null-environment")
                .await
                .unwrap(),
        )
        .await
        .unwrap();

    // Wait for the run to enter a successful status
    let ex_id = t
        .poll_for_execution_id(run.id, Duration::from_secs(5))
        .await;
    t.poll_for_status(ex_id, Successful, Duration::from_secs(180))
        .await;

    // Fetch and validate the log output
    let log_txt = t
        .get_text(format!("test-execution/{ex_id}/log.txt"))
        .await
        .unwrap();

    assert!(log_txt.ends_with("hello, world!\n"), "{log_txt:?}");

    // Fetch and validate the output.zip
    let mut zip = t.get_output_zip(ex_id).await.unwrap();

    assert_eq!(zip.rtf_output_content(), "hello, world!\n");
    assert!(zip.contains_path_prefix("output/logs/"));

    zip.read_json("output/events.json").unwrap();
    zip.read_json("output/resource-metrics.json").unwrap();

    assert!(
        !zip.contains_path_prefix("output/prometheus/environment/"),
        "shouldn't have any prometheus output from the environment"
    );
    assert_eq!(
        zip.read_prometheus_query("scenario", "scenario_up")
            .unwrap(),
        t.prometheus_query_with_namespace_filter(ex_id, "scenario_up", "up"),
    );
}

#[tokio::test]
#[serial]
async fn unknown_docker_image_in_environment_marks_execution_unrunnable() {
    let t = TestHelper::new();
    let run: TestRunSummary = t
        .json_post(
            "test-run/trigger",
            t.prepare_rep_payload("resources/test-plans/error/unknown-docker-image-env")
                .await
                .unwrap(),
        )
        .await
        .unwrap();

    let ex_id = t
        .poll_for_execution_id(run.id, Duration::from_secs(5))
        .await;
    let summary = t
        .poll_execution_for_terminal_status(ex_id, Duration::from_secs(180))
        .await;

    assert_eq!(summary.current_status, Unrunnable);

    // The Argo workflow pod watcher detected the bad image — the execution must have reached
    // the environment wait step but never moved on to the scenario phase.
    assert!(
        summary
            .status_history
            .iter()
            .any(|u| { u.status == Provisioning && u.message.as_deref() == Some(MSG_ARGO_WAIT) }),
        "expected environment Argo-wait step in history: {:#?}",
        summary.status_history,
    );
    assert!(
        !summary.status_history.iter().any(|u| u.status == Running),
        "scenario command must not have started: {:#?}",
        summary.status_history,
    );
}

#[tokio::test]
#[serial]
async fn unknown_docker_image_in_scenario_marks_execution_unrunnable() {
    let t = TestHelper::new();
    let run: TestRunSummary = t
        .json_post(
            "test-run/trigger",
            t.prepare_rep_payload("resources/test-plans/error/unknown-docker-image-scenario")
                .await
                .unwrap(),
        )
        .await
        .unwrap();

    let ex_id = t
        .poll_for_execution_id(run.id, Duration::from_secs(5))
        .await;
    let summary = t
        .poll_execution_for_terminal_status(ex_id, Duration::from_secs(180))
        .await;

    assert_eq!(summary.current_status, Unrunnable);

    // The scenario job's sidecar starts successfully (it can pull its own image), resolves the
    // scenario config, and sets the status to Running before kubernetes tries to pull the bad
    // scenario image. So Running must appear — its presence proves the environment provisioned
    // and the scenario job reached execution before the image pull failure was detected.
    assert!(
        summary.status_history.iter().any(|u| {
            u.status == EnvironmentReady && u.message.as_deref() == Some(MSG_JOB_WAIT)
        }),
        "expected scenario job-wait step in history: {:#?}",
        summary.status_history,
    );
    assert!(
        summary.status_history.iter().any(|u| u.status == Running),
        "expected Running in history (sidecar started before image pull failed): {:#?}",
        summary.status_history,
    );
}

// Valid admin emails are set based on the contents of the ../local-stack/k8s/admins.yaml file
#[test_case(Some("someone@test.com"), "user: someone@test.com"; "normal user")]
#[test_case(Some("admin@test.com"), "admin: admin@test.com"; "admin user")]
#[test_case(None, "unknown"; "header unset")]
#[tokio::test]
async fn whoami_correctly_identifies_the_user(email: Option<&'static str>, expected: &str) {
    let mut t = TestHelper::new();

    if let Some(email) = email {
        t.set_user_email(email);
    };

    let body = t.get_text("whoami").await.unwrap();

    assert_eq!(body, expected);
}
