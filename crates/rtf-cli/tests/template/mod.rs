use assert_cmd::Command;
use predicates::str::contains;
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
fn with_check_success(test_plan_dir: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg(format!("resources/valid/{test_plan_dir}/test-plan.yaml"))
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test]
fn with_values_from_cli_success() {
    // The sanity-check test plan defines values in the setup.provides
    // The only way to template successfully is to set this value from the cli
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
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
fn load_and_resolve_errors(file: &str, err_contains: &str) {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .arg("template")
        .arg(format!("resources/invalid/load-and-resolve/{file}"))
        .assert();

    res.stderr(contains(err_contains));
}
