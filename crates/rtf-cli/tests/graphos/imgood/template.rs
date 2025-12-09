use assert_cmd::Command;
use predicates::str::contains;
use simple_test_case::test_case;

#[test_case("graphos-canned-ops"; "canned ops")]
#[test_case("graphos-subgraph-router-url-overrides"; "subgraph router url overrides")]
#[test_case("graphos-subgraphs"; "subgraphs")]
#[test_case("graphos-supergraph"; "supergraph")]
#[ignore = "requires a valid GraphOS API Key for the imgood-observability-test graph"]
#[test]
fn check_completes(test_plan_dir: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg(format!("resources/test-plans/valid/{test_plan_dir}/test-plan.yaml"))
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}
