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

#[test]
#[ignore = "requires docker on the PATH"]
fn output_collection_completes_and_logs_warnings() {
    prepare_rtf_run("resources/test-plans/valid/output-collection")
        .assert()
        .success()
        .stderr(contains("environment has prometheus queries in its output_collection. These will not run when using `rtf run`"))
        .stderr(contains("scenario has output_collection defined. This will execute run when using `rtf run`"));
}
