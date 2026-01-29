use crate::common::prepare_rtf_run;
use predicates::str::contains;

#[test]
#[ignore = "requires docker on the PATH"]
fn docker_compose_environment_produces_expected_output() {
    prepare_rtf_run("resources/test-plans/valid/docker-compose-environment")
        .assert()
        .success()
        .stdout(contains("env: hello, world!"))
        .stdout(contains("SUCCESS: Environment variable echoed correctly"));
}

#[test]
#[ignore = "requires docker on the PATH"]
fn docker_compose_not_yaml_fails() {
    prepare_rtf_run("resources/test-plans/invalid/run/docker-compose-not-yaml")
        .assert()
        .failure()
        .stderr(contains("Unable to execute the docker compose up command"));
}

#[test]
#[ignore = "requires docker on the PATH"]
fn docker_compose_invalid_compose_fails() {
    prepare_rtf_run("resources/test-plans/invalid/run/docker-compose-invalid-compose")
        .assert()
        .failure()
        .stderr(contains("Unable to execute the docker compose up command"))
        .stderr(contains("not_services"));
}

#[test]
#[ignore = "requires docker on the PATH"]
fn docker_compose_invalid_image_fails() {
    prepare_rtf_run("resources/test-plans/invalid/run/docker-compose-invalid-image")
        .assert()
        .failure()
        .stderr(contains("Unable to execute the docker compose up command"))
        .stderr(contains("pull access denied"));
}

#[test]
#[ignore = "requires docker on the PATH"]
fn docker_compose_service_exits_fails() {
    prepare_rtf_run("resources/test-plans/invalid/run/docker-compose-service-exits")
        .assert()
        .failure()
        .stderr(contains("Unable to execute the docker compose up command"))
        .stderr(contains("exited (1)"));
}
