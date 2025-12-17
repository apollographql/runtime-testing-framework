use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
#[ignore = "requires a valid GraphOS API Key for the starstuff graph"]
fn check_completes_offline_license() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("template")
        .arg("resources/test-plans/valid/graphos-offline-license/test-plan.yaml")
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}
