//! The tests in this file make use of the GitHub API.
//!
//! You need to create an API token with access to the runtime-testing-framework repo in order to
//! complete the test. The API key should be set in the command for running the test e.g.
//! GITHUB_TOKEN=<YOUR_TOKEN_HERE> cargo test github -- --ignored

pub mod common;

use common::is_valid_test_plan;
use simple_test_case::test_case;

#[test_case("resources/github-file"; "github file")]
#[test_case("resources/github-config-files"; "github config files")]
#[test]
#[ignore = "requires a valid GitHub API Token"]
fn github_valid_test_plans(dir: &str) {
    is_valid_test_plan(dir);
}
