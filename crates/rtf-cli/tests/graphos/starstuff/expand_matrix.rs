use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;

#[test]
fn pretty_offline_license() {
    let test_plan_dir = "graphos-offline-license";

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("expand-matrix")
        .arg(format!("resources/test-plans/valid/{test_plan_dir}/test-plan.yaml"))
        .assert();

    let matrix_json_str =
        std::fs::read_to_string(format!("resources/test-plans/valid/{test_plan_dir}/matrix.json"))
            .expect("unable to load matrix.json");
    let expected_json: Value = serde_json::from_str(&matrix_json_str).unwrap();

    res.stdout(contains(
        serde_json::to_string_pretty(&expected_json).unwrap(),
    ));
}
