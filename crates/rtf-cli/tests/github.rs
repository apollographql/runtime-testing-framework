//! The tests in this file make use of the GitHub API.
//!
//! You need to create an API token with access to the runtime-testing-framework repo in order to
//! complete the test. The API key should be set in the command for running the test e.g.
//! GITHUB_TOKEN=<YOUR_TOKEN_HERE> cargo test github -- --ignored

pub mod common;

use assert_cmd::Command;
use assert_fs::{TempDir, prelude::PathChild};
use common::is_valid_test_plan;
use predicates::str::contains;
use simple_test_case::test_case;

#[test_case("resources/github-file"; "github file")]
#[test_case("resources/github-config-files"; "github config files")]
#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_valid_test_plans(dir: &str) {
    is_valid_test_plan(dir);
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn run_from_github_works() {
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
