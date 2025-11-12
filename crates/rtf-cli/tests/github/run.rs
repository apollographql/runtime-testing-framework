use crate::common::is_valid_test_plan;
use assert_cmd::Command;
use assert_fs::{TempDir, prelude::PathChild};
use predicates::str::contains;
use simple_test_case::test_case;

#[test_case("resources/valid/github-file"; "github file")]
#[test_case("resources/valid/github-config-files"; "github config files")]
#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_completes(dir: &str) {
    is_valid_test_plan(dir);
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_flag_produces_expected_output() {
    let temp = TempDir::new().unwrap();
    let outdir = temp.child("output");
    let outdir = outdir.path().to_str().unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    cmd.args([
        "run",
        "--github",
        "apollographql/runtime-testing-framework/example-test-plans/hello-world/test-plan.yaml",
        "--outdir",
        outdir,
    ])
    .assert()
    .success()
    .stdout(contains("hello, world!"));
}
