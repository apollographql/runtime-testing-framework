pub mod common;

use assert_cmd::Command;
use common::{is_valid_test_plan, prepare_rtf_run};
use predicates::str::contains;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    // Running with no args should return a help message to std_err
    let res = cmd.assert();

    // Check the output contains usage instructions for rtf
    res.stderr(contains("Usage: rtf"));
}

#[test]
fn run_command_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").assert();

    // Check the output contains usage instructions for rtf run
    res.stderr(contains("Usage: rtf run"));
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
