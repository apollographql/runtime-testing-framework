use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use simple_test_case::test_case;

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn check_completes_with_github_file() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("template")
        .arg("resources/test-plans/valid/github-file/test-plan.yaml")
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
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("template")
        .arg("resources/test-plans/valid/github-config-files/test-plan.yaml")
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
    let mut cmd = cargo_bin_cmd!("rtf");
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

#[test_case("file.txt"; "no forward slashes")]
#[test_case("path/file.txt"; "single forward slash")]
#[test]
// This test does not need to be ignored since it fails before an API token is required
fn github_flag_invalid_test_plan_path_fails(test_plan_path: &str) {
    let mut cmd = cargo_bin_cmd!("rtf");
    cmd.args(["template", "--github", test_plan_path, "--check"])
        .assert()
        .failure()
        .stderr(contains("GitHub uri must be in format ORG/REPO/PATH"));
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_invalid_path_fails() {
    let mut cmd = cargo_bin_cmd!("rtf");
    cmd.args([
        "template",
        "--github",
        "apollographql/runtime-testing-framework/not/a/valid/path/to/file.txt",
        "--check",
    ])
    .assert()
    .failure()
    .stderr(contains("Unable to load and resolve test plan from GitHub: HTTP status client error (404 Not Found) for url (https://api.github.com/repos/apollographql/runtime-testing-framework/contents/not/a/valid/path/to/file.txt)"));
}
