use crate::inline::{
    GRAPHOS_CANNED_OPS, GRAPHOS_SUBGRAPH_ROUTER_URL_OVERRIDES, GRAPHOS_SUBGRAPHS,
    GRAPHOS_SUPERGRAPH, RELATIVE_PATH, prepare_rtf_inline_all, prepare_rtf_inline_relative_files,
};
use simple_test_case::test_case;

const INLINED_OUTPUT_PATH: &str = "output/inlined-test-plan.yaml";

#[test_case("graphos-canned-ops"; "canned ops")]
#[test_case("graphos-subgraph-router-url-overrides"; "subgraph router url overrides")]
#[test_case("graphos-subgraphs"; "subgraphs")]
#[test_case("graphos-supergraph"; "supergraph")]
#[ignore = "requires a valid GraphOS API Key for the imgood-observability-test graph"]
#[test]
fn relative_files_succeeds(test_plan_dir: &str) {
    let mut cmd =
        prepare_rtf_inline_relative_files(&format!("resources/test-plans/valid/{test_plan_dir}"));
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
}

#[test]
#[ignore = "requires a valid GraphOS API Key for the imgood-observability-test graph"]
fn all_with_graphos_supergraph_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/graphos-supergraph");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, GRAPHOS_SUPERGRAPH);
}

#[test]
#[ignore = "requires a valid GraphOS API Key for the imgood-observability-test graph"]
fn all_with_graphos_subgraphs_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/graphos-subgraphs");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, GRAPHOS_SUBGRAPHS);
}

#[test]
#[ignore = "requires a valid GraphOS API Key for the imgood-observability-test graph"]
fn all_with_graphos_subgraph_router_url_overrides_succeeds() {
    let mut cmd =
        prepare_rtf_inline_all("resources/test-plans/valid/graphos-subgraph-router-url-overrides");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, GRAPHOS_SUBGRAPH_ROUTER_URL_OVERRIDES);
}

#[test]
#[ignore = "requires a valid GraphOS API Key for the imgood-observability-test graph"]
fn all_with_graphos_canned_ops_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/graphos-canned-ops");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, GRAPHOS_CANNED_OPS);
}
