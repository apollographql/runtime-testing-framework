use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;

#[ignore = "requires a valid GitHub API Token"]
#[test]
fn pretty_formats_config_files() {
    let test_plan_dir = "github-config-files";

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("expand-matrix")
        .arg(format!("resources/valid/{test_plan_dir}/test-plan.yaml"))
        .assert();

    let matrix_json_str =
        std::fs::read_to_string(format!("resources/valid/{test_plan_dir}/matrix.json"))
            .expect("unable to load matrix.json");
    let expected_json: Value = serde_json::from_str(&matrix_json_str).unwrap();

    res.stdout(contains(
        serde_json::to_string_pretty(&expected_json).unwrap(),
    ));
}

#[test]
fn pretty_formats_file() {
    let test_plan_dir = "github-file";

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("expand-matrix")
        .arg(format!("resources/valid/{test_plan_dir}/test-plan.yaml"))
        .assert();

    let matrix_json_str =
        std::fs::read_to_string(format!("resources/valid/{test_plan_dir}/matrix.json"))
            .expect("unable to load matrix.json");
    let expected_json: Value = serde_json::from_str(&matrix_json_str).unwrap();

    res.stdout(contains(
        serde_json::to_string_pretty(&expected_json).unwrap(),
    ));
}
