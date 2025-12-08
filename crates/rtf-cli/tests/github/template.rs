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

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_produces_expected_output() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    cmd.args([
        "template",
        "--github",
        "apollographql/runtime-testing-framework/example-test-plans/hello-world/test-plan.yaml",
        "--check",
    ])
    .assert()
    .success()
    .stdout(contains("name:"));
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_invalid_path_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    cmd.args([
        "template",
        "--github",
        "apollographql/runtime-testing-framework/not/a/valid/path/to/file.txt",
        "--check",
    ])
    .assert()
    .failure()
    .stderr(contains("HTTP status client error (404 Not Found) for url (https://api.github.com/repos/apollographql/runtime-testing-framework/contents/not/a/valid/path/to/file.txt)"));
}
