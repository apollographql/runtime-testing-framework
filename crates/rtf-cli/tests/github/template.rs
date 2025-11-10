use assert_cmd::Command;
use predicates::str::contains;

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn with_check_success_github_file() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("resources/valid/github-file/test-plan.yaml")
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn with_check_success_github_config_files() {
    // The sanity-check test plan defines values in the setup.provides
    // This test plan uses config from the sanity check
    // The only way to template successfully is to set this value from the cli
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("resources/valid/github-config-files/test-plan.yaml")
        .arg("--check")
        .arg("--value")
        .arg("setup_output=\"setup output\"")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}
