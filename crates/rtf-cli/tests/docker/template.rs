use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
fn check_completes_with_docker_scenario() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("template")
        .arg("resources/test-plans/valid/docker-scenario/test-plan.yaml")
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}
