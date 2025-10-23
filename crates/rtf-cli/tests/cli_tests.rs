pub mod common;

use assert_cmd::Command;
use common::{is_valid_test_plan, prepare_rtf_run};
use predicates::str::contains;
use serde_json::json;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    // Running with no args should return a help message to std_err
    let res = cmd.assert();

    // Check the output contains usage instructions for rtf
    res.stderr(contains("Usage: rtf"));
}

#[test]
fn expand_matrix_command_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("expand-matrix").assert();

    res.stderr(contains("Usage: rtf expand-matrix"));
}

#[test]
fn expand_matrix_command_invalid_test_plan_path_errors() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("expand-matrix").arg("/not/a/file.txt").assert();

    res.stderr(contains("No such file or directory (os error 2)"));
}

#[test]
fn expand_matrix_pretty_works() {
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
fn expand_matrix_compact_works() {
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

#[test]
fn run_command_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").assert();

    res.stderr(contains("no test plan provided"));
}

#[test]
fn run_command_invalid_test_plan_path_errors() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").arg("/not/a/file.txt").assert();

    res.stderr(contains("No such file or directory (os error 2)"));
}

#[test]
fn run_command_sanity_check_works() {
    is_valid_test_plan("resources/sanity-check");
}

#[test]
fn run_command_matrix_works() {
    is_valid_test_plan("resources/matrix-values");
}

#[test]
fn run_command_command_from_spec_works() {
    is_valid_test_plan("resources/command-from-spec");
}

#[test]
fn template_command_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("template").assert();

    res.stderr(contains("Usage: rtf template"));
}

#[test]
fn template_command_invalid_test_plan_path_errors() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("template").arg("/not/a/file.txt").assert();

    res.stderr(contains("No such file or directory (os error 2)"));
}

#[test]
fn overriding_values_works() {
    // default echo arg should be foo
    prepare_rtf_run("resources/value-overrides")
        .assert()
        .success()
        .stdout(contains("foo"));

    // specifying as a command line value should override
    prepare_rtf_run("resources/value-overrides")
        .arg("--value")
        .arg("echo_me=bar")
        .assert()
        .success()
        .stdout(contains("bar"));

    // values.json should override to baz
    prepare_rtf_run("resources/value-overrides")
        .arg("--values")
        .arg("resources/value-overrides/values.json")
        .assert()
        .success()
        .stdout(contains("baz"));
}

#[test]
fn resolved_values_provider_works() {
    prepare_rtf_run("resources/resolved-values")
        .assert()
        .success()
        .stdout(contains(r#""foo":"bar""#));

    prepare_rtf_run("resources/resolved-values")
        .arg("--value")
        .arg("foo=baz")
        .assert()
        .success()
        .stdout(contains(r#""foo":"baz""#));

    prepare_rtf_run("resources/resolved-values")
        .arg("--value")
        .arg("echo_me=baz")
        .assert()
        .success()
        .stdout(contains(r#""foo":"bar""#))
        .stdout(contains(r#""echo_me":"baz""#));
}

#[test]
fn custom_matrix_variant_names_work() {
    let mut cmd = prepare_rtf_run("resources/custom-matrix-variant-names");
    cmd.assert().success();
    cmd.assert_path_exists("output/world!-mother");
    cmd.assert_path_exists("output/world!-father");
    cmd.assert_path_exists("output/sailor-mother");
    cmd.assert_path_exists("output/sailor-father");
}

#[test]
fn matrix_include_works() {
    prepare_rtf_run("resources/matrix-include")
        .assert()
        .success()
        .stdout(contains("hello, world!"))
        .stdout(contains("hello, sailor"))
        .stdout(contains("hello, mother"))
        .stdout(contains("hello, father"))
        .stdout(contains("what a wonderful world!"))
        .stdout(contains("what a wonderful sailor"))
        .stdout(contains("what a wonderful mother"))
        .stdout(contains("what a wonderful father"));
}
