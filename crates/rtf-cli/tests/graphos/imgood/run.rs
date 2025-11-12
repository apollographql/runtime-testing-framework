//! The tests in this file make use of the starstuff graph.
//! The tests in this file make use of the imgood-observability-test graph, current variant in the
//! apollo-team-runtime-readiness
//!
//! You need to create an API Key with Graph Admin permissions for this graph to successfully
//! complete the test. The API key should be set in the command for running the test e.g.
//! APOLLO_KEY=<YOUR_KEY_HERE> cargo test imgood_observability -- --ignored

use crate::common::is_valid_test_plan;
use simple_test_case::test_case;

#[test_case("resources/valid/graphos-supergraph"; "supergraph sdl")]
#[test_case("resources/valid/graphos-subgraph-router-url-overrides"; "subgraph router url overrides")]
#[test_case("resources/valid/graphos-subgraphs"; "subgraph sdls")]
#[test_case("resources/valid/graphos-canned-ops"; "canned operations")]
#[test]
#[ignore = "requires a valid GraphOS API Key for the imgood-observability-test graph"]
fn completes(dir: &str) {
    is_valid_test_plan(dir);
}
