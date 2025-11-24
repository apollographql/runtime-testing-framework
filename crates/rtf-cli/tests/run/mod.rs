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
fn basic_completes() {
    is_valid_test_plan("resources/valid/sanity-check");
}

#[test]
fn backwards_compatible_variable_config() {
    is_valid_test_plan("resources/valid/backwards-compatible-variable-config");
}

#[test]
fn matrix_completes() {
    is_valid_test_plan("resources/valid/matrix-variables");
}

#[test]
fn command_from_spec_completes() {
    is_valid_test_plan("resources/valid/command-from-spec");
}

#[test]
fn custom_provider_default_value_completes() {
    prepare_rtf_run("resources/valid/custom-provider-default-value")
        .assert()
        .success()
        .stdout(contains("default custom provider text file"));
}

#[test]
fn custom_provider_templated_variable_completes() {
    prepare_rtf_run("resources/valid/custom-provider-templated-variable")
        .assert()
        .success()
        .stdout(contains("scenario specified custom provider text file"));
}

#[test]
fn variables_override_works() {
    // default echo arg should be foo
    prepare_rtf_run("resources/valid/variable-overrides")
        .assert()
        .success()
        .stdout(contains("foo"));

    // specifying as a command line variable should override
    prepare_rtf_run("resources/valid/variable-overrides")
        .arg("--var")
        .arg("echo_me=bar")
        .assert()
        .success()
        .stdout(contains("bar"));

    // variables.json should override to baz
    prepare_rtf_run("resources/valid/variable-overrides")
        .arg("--vars")
        .arg("resources/valid/variable-overrides/variables.json")
        .assert()
        .success()
        .stdout(contains("baz"));
}

#[test]
fn variables_override_with_backwards_compatible_flag_works() {
    // default echo arg should be foo
    prepare_rtf_run("resources/valid/variable-overrides")
        .assert()
        .success()
        .stdout(contains("foo"));

    // specifying as a command line variable should override
    prepare_rtf_run("resources/valid/variable-overrides")
        .arg("--value")
        .arg("echo_me=bar")
        .assert()
        .success()
        .stdout(contains("bar"));

    // variables.json should override to baz
    prepare_rtf_run("resources/valid/variable-overrides")
        .arg("--values")
        .arg("resources/valid/variable-overrides/variables.json")
        .assert()
        .success()
        .stdout(contains("baz"));
}

#[test]
fn regression_relative_path_from_variable() {
    // When using a variable from the test plan we should resolve relative to the directory
    // containing the test plan
    prepare_rtf_run("resources/valid/regression-relative-path-from-template-variable")
        .assert()
        .success()
        .stdout(contains("from test plan dir"));

    // When using a variable from variables.json we should resolve relative to the directory
    // containing the variables file
    prepare_rtf_run("resources/valid/regression-relative-path-from-template-variable")
        .arg("--vars")
        .arg("resources/valid/regression-relative-path-from-template-variable/variables-dir/variables.json")
        .assert()
        .success()
        .stdout(contains("from variables.json dir"));

    // When using a command line variable we should resolve relative to the current working directory
    let mut cmd =
        prepare_rtf_run("resources/valid/regression-relative-path-from-template-variable");
    let dir = cmd.child_path("cli-working-dir");

    cmd.current_dir(dir)
        .arg("--var")
        .arg("cat_path=cat-me.txt")
        .assert()
        .success()
        .stdout(contains("from cli working dir"));
}

#[test]
fn matrix_custom_variant_names_work() {
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

#[test_case("setup-execution-fails", "Unable to execute the setup.sh command:"; "setup script execution fails")]
#[test_case("setup-file-provider-fails", "Unable to resolve and write FROG_GIF file: stream did not contain valid UTF-8"; "setup file provider fails")]
#[test_case("setup-provides-not-json", "Environment setup output not valid json: \"not valid json output\\n\""; "setup output not json")]
#[test_case("setup-provides-missing-key", "Missing required output fields from environment setup: [\"setup_output\"]"; "setup missing required output")]
#[test_case("scenario-execution-fails", "Unable to execute the scenario.sh command:"; "scenario script execution fails")]
#[test_case("teardown-execution-fails", "Unable to execute the teardown.sh command:"; "teardown script execution fails")]
#[test]
fn execution_fails(test_plan_dir: &str, expected_err: &str) {
    prepare_rtf_run(&format!("resources/invalid/run/{test_plan_dir}"))
        .assert()
        .failure()
        .stderr(contains(expected_err));
}

#[test]
fn load_and_resolve_from_invalid_github_uri_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("run")
        .arg("--github")
        .arg("not a valid github uri")
        .assert();

    res.failure().stderr(contains("invalid GitHub uri: \"not a valid github uri\" - GitHub uri must be in format ORG/REPO/PATH"));
}

#[test]
fn load_and_resolve_from_github_missing_token_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("run")
        .arg("--github")
        .arg("org/repo/path")
        .assert();

    res.failure().stderr(contains("no GitHub client available"));
}
