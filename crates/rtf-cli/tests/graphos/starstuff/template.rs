use assert_cmd::Command;
use predicates::str::contains;

#[test]
#[ignore = "requires a valid GraphOS API Key for the starstuff graph"]
fn check_completes_offline_license() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg("resources/valid/graphos-offline-license/test-plan.yaml")
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}
