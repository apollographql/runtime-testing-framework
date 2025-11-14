use assert_cmd::Command;
use predicates::str::contains;

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn check_completes_with_github_file() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg("resources/valid/github-file/test-plan.yaml")
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn check_completes_with_github_config_files() {
    // The sanity-check test plan defines variables in the setup.provides
    // This test plan uses config from the sanity check
    // The only way to template successfully is to set this variable from the cli
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg("resources/valid/github-config-files/test-plan.yaml")
        .arg("--check")
        .arg("--var")
        .arg("setup_output=\"setup output\"")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}
