use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;
use simple_test_case::test_case;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("expand-matrix").assert();

    res.stderr(contains("Usage: rtf expand-matrix"));
}

#[test_case("command-from-spec"; "command from spec")]
#[test_case("custom-matrix-variant-names"; "custom matrix variant names")]
#[test_case("matrix-include"; "matrix include")]
#[test_case("matrix-values"; "matrix values")]
#[test_case("sanity-check"; "sanity check")]
#[test_case("resolved-values"; "resolved-values")]
#[test_case("value-overrides"; "value overrides")]
#[test]
fn pretty_formats_correctly(test_plan_dir: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
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

#[test_case("command-from-spec"; "command from spec")]
#[test_case("custom-matrix-variant-names"; "custom matrix variant names")]
#[test_case("matrix-include"; "matrix include")]
#[test_case("matrix-values"; "matrix values")]
#[test_case("sanity-check"; "sanity check")]
#[test_case("resolved-values"; "resolved-values")]
#[test_case("value-overrides"; "value overrides")]
#[test]
fn compact_formats_correctly(test_plan_dir: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("expand-matrix")
        .arg(format!("resources/valid/{test_plan_dir}/test-plan.yaml"))
        .arg("--compact")
        .assert();

    let matrix_json_str =
        std::fs::read_to_string(format!("resources/valid/{test_plan_dir}/matrix.json"))
            .expect("unable to load matrix.json");
    let expected_json: Value = serde_json::from_str(&matrix_json_str).unwrap();

    res.stdout(contains(serde_json::to_string(&expected_json).unwrap()));
}

#[test]
fn duplicate_variant_names_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("expand-matrix")
        .arg("resources/invalid/expand-matrix/duplicate-variant-names.yaml")
        .arg("--compact")
        .assert();

    res.failure().stderr(contains(
        "The provided variant_names template produced duplicate names: [\"foo\"]",
    ));
}
