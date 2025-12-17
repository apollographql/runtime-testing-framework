use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;
use serde_json::Value;
use simple_test_case::test_case;

#[test_case("graphos-canned-ops"; "canned ops")]
#[test_case("graphos-subgraph-router-url-overrides"; "subgraph router url overrides")]
#[test_case("graphos-subgraphs"; "subgraphs")]
#[test_case("graphos-supergraph"; "supergraph")]
#[test]
fn pretty(test_plan_dir: &str) {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("expand-matrix")
        .arg(format!(
            "resources/test-plans/valid/{test_plan_dir}/test-plan.yaml"
        ))
        .assert();

    let matrix_json_str = std::fs::read_to_string(format!(
        "resources/test-plans/valid/{test_plan_dir}/matrix.json"
    ))
    .expect("unable to load matrix.json");
    let expected_json: Value = serde_json::from_str(&matrix_json_str).unwrap();

    res.stdout(contains(
        serde_json::to_string_pretty(&expected_json).unwrap(),
    ));
}
