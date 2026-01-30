use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
fn check_completes_with_docker_compose_environment() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("template")
        .arg("resources/test-plans/valid/docker-compose-environment/test-plan.yaml")
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test]
fn check_completes_with_docker_compose_inline_dir() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("template")
        .arg("resources/test-plans/valid/docker-compose-inline-dir/test-plan.yaml")
        .arg("--check")
        .assert();

    // Check that InlineDir compose files parse and template correctly
    res.success()
        .stdout(contains("name:"))
        .stdout(contains("inline_dir"))
        .stdout(contains("base.yaml"))
        .stdout(contains("overlay.yaml"));
}
