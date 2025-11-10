use crate::common::{is_valid_test_plan, prepare_rtf_run};
use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").assert();

    res.stderr(contains("no test plan provided"));
}

#[test]
fn sanity_check_success() {
    is_valid_test_plan("resources/valid/sanity-check");
}

#[test]
fn matrix_success() {
    is_valid_test_plan("resources/valid/matrix-values");
}

#[test]
fn command_from_spec_success() {
    is_valid_test_plan("resources/valid/command-from-spec");
}

#[test]
fn overriding_values_success() {
    // default echo arg should be foo
    prepare_rtf_run("resources/valid/value-overrides")
        .assert()
        .success()
        .stdout(contains("foo"));

    // specifying as a command line value should override
    prepare_rtf_run("resources/valid/value-overrides")
        .arg("--value")
        .arg("echo_me=bar")
        .assert()
        .success()
        .stdout(contains("bar"));

    // values.json should override to baz
    prepare_rtf_run("resources/valid/value-overrides")
        .arg("--values")
        .arg("resources/valid/value-overrides/values.json")
        .assert()
        .success()
        .stdout(contains("baz"));
}

#[test]
fn resolved_values_provider_success() {
    prepare_rtf_run("resources/valid/resolved-values")
        .assert()
        .success()
        .stdout(contains(r#""foo":"bar""#));

    prepare_rtf_run("resources/valid/resolved-values")
        .arg("--value")
        .arg("foo=baz")
        .assert()
        .success()
        .stdout(contains(r#""foo":"baz""#));

    prepare_rtf_run("resources/valid/resolved-values")
        .arg("--value")
        .arg("echo_me=baz")
        .assert()
        .success()
        .stdout(contains(r#""foo":"bar""#))
        .stdout(contains(r#""echo_me":"baz""#));
}

#[test]
fn custom_matrix_variant_names_work() {
    let mut cmd = prepare_rtf_run("resources/valid/custom-matrix-variant-names");
    cmd.assert().success();
    cmd.assert_path_exists("output/world!-mother");
    cmd.assert_path_exists("output/world!-father");
    cmd.assert_path_exists("output/sailor-mother");
    cmd.assert_path_exists("output/sailor-father");
}

#[test]
fn matrix_include_success() {
    prepare_rtf_run("resources/valid/matrix-include")
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
