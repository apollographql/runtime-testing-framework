use assert_cmd::Command;
use predicates::str::contains;
use serde_json::json;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("expand-matrix").assert();

    res.stderr(contains("Usage: rtf expand-matrix"));
}

#[test]
fn invalid_test_plan_path_errors() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("expand-matrix").arg("/not/a/file.txt").assert();

    res.stderr(contains("No such file or directory (os error 2)"));
}

#[test]
fn pretty() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("expand-matrix")
        .arg("resources/sanity-check/test-plan.yaml")
        .assert();

    let expected_json = json!({
        "variants": [{
            "name": "matrix_variant_1",
            "values": {"message": "hello, world!"}
        }]
    });

    res.stdout(contains(
        serde_json::to_string_pretty(&expected_json).unwrap(),
    ));
}

#[test]
fn compact() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("expand-matrix")
        .arg("resources/sanity-check/test-plan.yaml")
        .arg("--compact")
        .assert();

    let expected_json = json!({
        "variants": [{
            "name": "matrix_variant_1",
            "values": {"message": "hello, world!"}
        }]
    });

    res.stdout(contains(serde_json::to_string(&expected_json).unwrap()));
}
