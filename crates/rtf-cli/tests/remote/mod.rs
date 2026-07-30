use crate::common::{prepare_for_test, prepare_rtf_remote_prepare};
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::{boolean::PredicateBooleanExt, str::contains};

const FIXTURE: &str = "resources/test-plans/valid/remote-prepare";
const DUMMY_ID: &str = "11111111-1111-1111-1111-111111111111";

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
fn remote_prepare_fails_with_incompatible_environment() {
    // docker-scenario fixture has a script environment + docker scenario
    let mut cmd = prepare_rtf_remote_prepare("resources/test-plans/valid/docker-scenario");
    cmd.assert().failure().stderr(contains(
        "expected null or docker-compose environment when running via the Orchestrator",
    ));
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
fn remote_request_bad_orchestrator_url_env_var_fails() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "request", "/health"])
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
}

#[test]
fn remote_request_orchestrator_url_env_var_valid_url_is_used() {
    // A syntactically valid custom URL should be accepted and used as the base URL, so the
    // command should get past URL resolution and fail for an unrelated reason (no GCP ADC
    // credentials on the test machine) rather than an "invalid URL" error.
    cargo_bin_cmd!("rtf")
        .args(["remote", "request", "/health"])
        .env("RTF_ORCHESTRATOR_URL", "https://example.invalid")
        .assert()
        .failure()
        .stderr(contains("invalid URL").not());
}

#[test]
fn remote_run_bad_orchestrator_url_env_var_fails() {
    // `remote run` proves the env var override applies to every `remote` subcommand, not just
    // `request` — there is no per-subcommand flag, only `OrchestratorClient::new()`.
    let test_setup = prepare_for_test(FIXTURE);
    cargo_bin_cmd!("rtf")
        .arg("remote")
        .arg("run")
        .arg(&test_setup.test_plan_file_path)
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
}

#[test]
fn remote_ci_run_bad_orchestrator_url_env_var_fails() {
    let test_setup = prepare_for_test(FIXTURE);
    cargo_bin_cmd!("rtf")
        .arg("remote")
        .arg("ci-run")
        .arg(&test_setup.test_plan_file_path)
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
}

#[test]
fn remote_execution_log_bad_orchestrator_url_env_var_fails() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "execution-log", DUMMY_ID])
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
}

#[test]
fn remote_execution_status_bad_orchestrator_url_env_var_fails() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "execution-status", DUMMY_ID])
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
}

#[test]
fn remote_run_status_bad_orchestrator_url_env_var_fails() {
    cargo_bin_cmd!("rtf")
        .args(["remote", "run-status", DUMMY_ID])
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
}

#[test]
fn remote_execution_output_bad_orchestrator_url_env_var_fails() {
    let test_setup = prepare_for_test(FIXTURE);
    cargo_bin_cmd!("rtf")
        .args(["remote", "execution-output", DUMMY_ID])
        .arg("--outdir")
        .arg(&test_setup.output_file_path)
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
}

#[test]
fn remote_run_output_bad_orchestrator_url_env_var_fails() {
    let test_setup = prepare_for_test(FIXTURE);
    cargo_bin_cmd!("rtf")
        .args(["remote", "run-output", DUMMY_ID])
        .arg("--outdir")
        .arg(&test_setup.output_file_path)
        .env("RTF_ORCHESTRATOR_URL", "not-a-url")
        .assert()
        .failure()
        .stderr(contains("invalid URL"));
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
