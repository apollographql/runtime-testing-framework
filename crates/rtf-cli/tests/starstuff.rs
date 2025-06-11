//! The tests in this file make use of the starstuff graph.
//!
//! You need to create an API Key with Graph Admin permissions for this graph to successfully
//! complete the test. The API key should be set in the command for running the test e.g.
//! APOLLO_KEY=<YOUR_KEY_HERE> cargo test starstuff -- --ignored

pub mod common;

use common::is_valid_test_plan;
use simple_test_case::test_case;

#[test_case("resources/graphos-offline-license"; "offline license")]
#[test]
#[ignore = "requires a valid GraphOS API Key"]
fn starstuff_valid_test_plans(dir: &str) {
    is_valid_test_plan(dir);
}
