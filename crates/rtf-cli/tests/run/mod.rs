use crate::common::{is_valid_test_plan, prepare_rtf_run};
use assert_cmd::Command;
use predicates::str::contains;
use simple_test_case::test_case;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").assert();

    res.stderr(contains("no test plan provided"));
}

#[test]
fn completes_sanity_check() {
    is_valid_test_plan("resources/valid/sanity-check");
}

#[test]
fn completes_with_matrix() {
    is_valid_test_plan("resources/valid/matrix-values");
}

#[test]
fn completes_command_from_spec() {
    is_valid_test_plan("resources/valid/command-from-spec");
}

#[test]
fn value_overriding_works() {
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
fn override_resolved_values_works() {
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
fn matrix_include_completes() {
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

#[test_case("setup-execution-fails", "Unable to execute the setup.sh command:"; "setup execution fails")]
#[test_case("setup-file-provider-fails", "Unable to resolve and write FROG_GIF file: stream did not contain valid UTF-8"; "setup provider error")]
#[test_case("setup-provides-not-json", "Environment setup output not valid json: \"not valid json output\\n\""; "setup provides not json")]
#[test_case("setup-provides-missing-key", "Missing required output fields from environment setup: [\"setup_output\"]"; "setup provides missing key")]
#[test_case("scenario-execution-fails", "Unable to execute the scenario.sh command:"; "scenario execution fails")]
#[test_case("teardown-execution-fails", "Unable to execute the teardown.sh command:"; "teardown execution fails")]
#[test]
fn execution_fails(test_plan_dir: &str, expected_err: &str) {
    prepare_rtf_run(&format!("resources/invalid/run/{test_plan_dir}"))
        .assert()
        .failure()
        .stderr(contains(expected_err));
}
