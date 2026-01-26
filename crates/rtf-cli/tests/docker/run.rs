use crate::common::prepare_rtf_run;
use predicates::str::contains;

#[test]
#[ignore = "requires docker on the PATH"]
fn docker_scenario_produces_expected_output() {
    prepare_rtf_run("resources/test-plans/valid/docker-scenario")
        .assert()
        .success()
        .stdout(contains("env-setup :: hello, world!"))
        .stdout(contains("scenario :: hello, world!"))
        .stdout(contains("env-teardown :: hello, world!"));
}
