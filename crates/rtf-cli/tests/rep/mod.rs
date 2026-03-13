use crate::common::prepare_rtf_rep_prepare;
use assert_fs::prelude::*;
use predicates::str::contains;

const FIXTURE: &str = "resources/test-plans/valid/rep-prepare";
const OUTPUT_FILE: &str = "output/rep-test-plan.json";

#[test]
fn rep_prepare_produces_output_file() {
    let mut cmd = prepare_rtf_rep_prepare(FIXTURE);
    cmd.assert().success();
    cmd.assert_path_exists(OUTPUT_FILE);
}

#[test]
fn rep_prepare_output_contains_relative_file_content() {
    let mut cmd = prepare_rtf_rep_prepare(FIXTURE);
    cmd.assert().success();
    cmd.assert_file_contains(OUTPUT_FILE, "alpine:latest");
}

#[test]
fn rep_prepare_fails_with_existing_outdir() {
    let mut cmd = prepare_rtf_rep_prepare(FIXTURE);
    let out_dir = cmd.child("output");
    out_dir.create_dir_all().unwrap();
    out_dir.child("existing.txt").write_str("content").unwrap();

    cmd.assert()
        .failure()
        .stderr(contains("already exists and is non-empty"));
}

#[test]
fn rep_prepare_force_succeeds_with_existing_outdir() {
    let mut cmd = prepare_rtf_rep_prepare(FIXTURE);
    let out_dir = cmd.child("output");
    out_dir.create_dir_all().unwrap();
    out_dir.child("existing.txt").write_str("content").unwrap();

    cmd.arg("--force").assert().success();
    cmd.assert_path_exists(OUTPUT_FILE);
}

#[test]
fn rep_prepare_fails_with_non_docker_compose_environment() {
    // docker-scenario fixture has a script environment + docker scenario
    let mut cmd = prepare_rtf_rep_prepare("resources/test-plans/valid/docker-scenario");
    cmd.assert()
        .failure()
        .stderr(contains("DockerComposeEnvironment"));
}

#[test]
fn rep_prepare_fails_with_non_docker_scenario() {
    // docker-compose-environment fixture has a docker-compose env + script scenario
    let mut cmd = prepare_rtf_rep_prepare("resources/test-plans/valid/docker-compose-environment");
    cmd.assert().failure().stderr(contains("DockerScenario"));
}

#[test]
fn rep_prepare_fails_with_both_wrong_reports_both_errors() {
    // sanity-check fixture has a script environment + script scenario
    let mut cmd = prepare_rtf_rep_prepare("resources/test-plans/valid/sanity-check");
    cmd.assert()
        .failure()
        .stderr(contains("DockerComposeEnvironment"))
        .stderr(contains("DockerScenario"));
}
