use assert_cmd::Command;
use indoc::indoc;
use predicates::str::{contains, is_match};
use simple_test_case::test_case;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("template").assert();

    res.stderr(contains("Usage: rtf template"));
}

#[test_case("command-from-spec"; "command from spec")]
#[test_case("custom-matrix-variant-names"; "custom matrix variant names")]
#[test_case("matrix-include"; "matrix include")]
#[test_case("matrix-values"; "matrix values")]
#[test_case("resolved-values"; "resolved values")]
#[test_case("value-overrides"; "value overrides")]
// Template and check all valid test plans except for the sanity check (which requires a provides value)
// and the github and graphos test plans which are tested in their respective modules
#[test]
fn check_completes_basic(test_plan_dir: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!("resources/valid/{test_plan_dir}/test-plan.yaml"))
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test]
fn check_completes_with_cli_values() {
    // The sanity-check test plan defines values in the setup.provides
    // The only way to template successfully is to set this value from the cli
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("resources/valid/sanity-check/test-plan.yaml")
        .arg("--check")
        .arg("--value")
        .arg("setup_output=\"setup output\"")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test_case(
    "does-not-exist.yaml",
    "No such file or directory (os error 2)";
    "file does not exist"
)]
#[test_case(
    "not-yaml.txt",
    "Unable to parse test plan yaml: invalid type: string \"This is not a yaml file\", expected struct RawTestPlanConfig";
    "file is not yaml"
)]
#[test_case(
    "not-a-test-plan.yaml",
    "Unable to parse test plan yaml: missing field `name`";
    "file not test plan yaml"
)]
#[test_case(
    "missing-required-field.yaml",
    "Unable to parse test plan yaml: missing field `scenario`";
    "test plan missing required fields"
)]
#[test_case(
    "invalid-config-spec.yaml",
    "Unable to parse test plan yaml: expected valid inline config section or from with overrides";
    "invalid config spec"
)]
#[test_case(
    "malformed-environment.yaml",
    "malformed environment config section";
    "malformed environment"
)]
#[test_case(
    "malformed-scenario.yaml",
    "malformed scenario config section";
    "malformed scenario"
)]
#[test]
fn load_and_resolve_fails(file: &str, err_contains: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!("resources/invalid/load-and-resolve/{file}"))
        .assert();

    res.stderr(contains(err_contains));
}

#[test_case(
    "conflicting-keys.yaml",
    "(test_plan) Conflicting value and matrix definitions\nfoo";
    "conflicting keys"
)]
#[test_case(
    "empty-matrix.yaml",
    "(test_plan) Empty array for matrix value\nfoo";
    "empty matrix"
)]
#[test_case(
    "inconsistent-matrix-values.yaml",
    "(test_plan) Inconsistent types for matrix value\nfoo";
    "inconsistent matrix values"
)]
#[test_case(
    "inconsistent-matrix-include.yaml",
    "(test_plan) Inconsistent types for matrix include maps\nmatrix include maps must share consistent keys and types";
    "inconsistent matrix include"
)]
#[test_case(
    "missing-values.yaml",
    indoc!(r#"
    (environment.setup) Missing template values definitions. Make sure the value is defined in the scenario or environment config values
      - bar: ""
    
    (environment.teardown) Missing template values definitions. Make sure the value is defined in the scenario or environment config values
      - baz: ""
    
    (scenario) Missing template values definitions. Make sure the value is defined in the scenario or environment config values
      - foo: ""
    "#);
    "missing values"
)]
#[test_case(
    "unknown-values.yaml",
    "(environment.teardown.env_vars.FOO) Unknown templating value. Make sure a value is defined for this value to resolve to.\nfoo";
    "unknown values"
)]
#[test]
fn templating_fails(file: &str, err_contains: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!("resources/invalid/templating/{file}"))
        .assert();

    res.stderr(contains(format!("Templating failed\n{err_contains}")));
}

#[test]
fn duplicate_variant_names_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg("resources/invalid/templating/duplicate-variant-names.yaml")
        .assert();

    res.stderr(contains(
        "The provided variant_names template produced duplicate names: [\"foo\"]",
    ));
}

#[test_case(
    "missing-graphos-key.yaml",
    "(scenario.LICENSE_FILE) No API key provided for calling the Apollo GraphOS API\nexpected os env key APOLLO_KEY";
    "missing graphos key"
)]
#[test_case(
    "missing-github-key.yaml",
    "(scenario.RTF_README) No API key provided for calling the GitHub API\nexpected os env key GITHUB_TOKEN";
    "missing github key"
)]
#[test_case(
    "required-file.yaml",
    "(scenario.REQUIRED) A required file has not been defined\nthis will cause a check failure";
    "required file"
)]
#[test_case(
    "duplicate-value-names.yaml",
    "(scenario.values) Non-unique value names found\nfoo";
    "duplicate value names"
)]
#[test_case(
    "duplicate-env-vars.yaml",
    "(scenario) Non-unique environment variables found\nFOO";
    "duplicate env vars"
)]
#[test]
fn check_fails(file: &str, err_contains: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!("resources/invalid/checks/{file}"))
        .arg("--check")
        .assert();

    res.stderr(contains(format!(
        "Static analysis checks failed\n{err_contains}"
    )));
}

// We need to test the error from relative file using a regex match as the error message contains
// the absolute path to the missing file which will be different on each system that runs the test
#[test]
fn check_fails_missing_relative_file() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("resources/invalid/checks/missing-relative-file.yaml")
        .arg("--check")
        .assert();

    res.stderr(
        is_match(
            r#"Static analysis checks failed
\(scenario\.command\.command_provider\) The requested file did not exist
provided path was file://.*/resources/invalid/checks/does-not-exist\.sh"#,
        )
        .unwrap(),
    );
}
