use crate::common::prepare_rtf_rep_prepare;
use predicates::str::contains;

const FIXTURE: &str = "resources/test-plans/valid/rep-prepare";

#[test]
fn rep_prepare_is_successful() {
    let mut cmd = prepare_rtf_rep_prepare(FIXTURE);
    cmd.assert().success();
}

#[test]
fn rep_prepare_output_contains_relative_file_content() {
    let mut cmd = prepare_rtf_rep_prepare(FIXTURE);
    cmd.assert().success().stdout(contains("alpine:latest"));
}

#[test]
fn rep_prepare_fails_with_non_docker_compose_environment() {
    // docker-scenario fixture has a script environment + docker scenario
    let mut cmd = prepare_rtf_rep_prepare("resources/test-plans/valid/docker-scenario");
    cmd.assert()
        .failure()
        .stderr(contains("missing field `compose_files`"));
}

#[test]
fn rep_prepare_fails_with_non_docker_scenario() {
    // docker-compose-environment fixture has a docker-compose env + script scenario
    let mut cmd = prepare_rtf_rep_prepare("resources/test-plans/valid/docker-compose-environment");
    cmd.assert()
        .failure()
        .stderr(contains("missing field `docker`"));
}
