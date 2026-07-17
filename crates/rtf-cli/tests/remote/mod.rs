use crate::common::{prepare_for_test, prepare_rtf_remote_prepare};
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

const FIXTURE: &str = "resources/test-plans/valid/remote-prepare";

#[test]
fn remote_prepare_is_successful() {
    let mut cmd = prepare_rtf_remote_prepare(FIXTURE);
    cmd.assert().success();
}

#[test]
fn remote_prepare_output_contains_relative_file_content() {
    let mut cmd = prepare_rtf_remote_prepare(FIXTURE);
    cmd.assert().success().stdout(contains("alpine:latest"));
}

#[test]
fn remote_prepare_fails_with_non_docker_compose_environment() {
    // docker-scenario fixture has a script environment + docker scenario
    let mut cmd = prepare_rtf_remote_prepare("resources/test-plans/valid/docker-scenario");
    cmd.assert()
        .failure()
        .stderr(contains("missing field `compose_files`"));
}

#[test]
fn remote_prepare_fails_with_non_docker_scenario() {
    // docker-compose-environment fixture has a docker-compose env + script scenario
    let mut cmd =
        prepare_rtf_remote_prepare("resources/test-plans/valid/docker-compose-environment");
    cmd.assert()
        .failure()
        .stderr(contains("missing field `docker`"));
}

#[test]
fn remote_request_help_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "request", "--help"])
        .assert()
        .success();
}

#[test]
fn remote_request_bad_orchestrator_url_fails() {
    cargo_bin_cmd!("rtf")
        .args([
            "remote",
            "request",
            "/health",
            "--orchestrator-url",
            "not-a-url",
        ])
        .assert()
        .failure();
}

#[test]
#[ignore = "requires GCP Application Default Credentials and Secret Manager access"]
fn remote_request_health_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "request", "/health"])
        .assert()
        .success();
}

#[test]
fn remote_run_help_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "run", "--help"])
        .assert()
        .success();
}

#[test]
#[ignore = "requires GCP Application Default Credentials and Secret Manager access"]
fn remote_run_smoke_test_succeeds() {
    cargo_bin_cmd!("rtf")
        .args([
            "remote",
            "run",
            "../rep-orchestrator/resources/test-plans/valid/smoke/test-plan.yaml",
        ])
        .assert()
        .success()
        .stdout(contains(
            "View test run status: https://api.rtf.apollographql.com/ui/run/",
        ));
}

#[test]
fn remote_ci_run_help_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "ci-run", "--help"])
        .assert()
        .success();
}

#[test]
#[ignore = "requires GCP Application Default Credentials and Secret Manager access"]
fn remote_ci_run_smoke_test_succeeds() {
    cargo_bin_cmd!("rtf")
        .args([
            "remote",
            "ci-run",
            "../rep-orchestrator/resources/test-plans/valid/smoke/test-plan.yaml",
        ])
        .assert()
        .success();
}

#[test]
fn remote_execution_output_help_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "execution-output", "--help"])
        .assert()
        .success();
}

#[test]
fn remote_run_output_help_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "run-output", "--help"])
        .assert()
        .success();
}

#[test]
fn rep_prepare_is_successful_and_shows_warning() {
    let test_setup = prepare_for_test(FIXTURE);
    let mut cmd = cargo_bin_cmd!("rtf");

    cmd.arg("rep")
        .arg("prepare")
        .arg(&test_setup.test_plan_file_path)
        .arg("-vv");

    cmd.assert().success().stderr(contains(
        "`rtf rep` is deprecated and will be removed in a future release; use `rtf remote` instead",
    ));
}
