use assert_cmd::Command;
use predicates::str::contains;
use simple_test_case::test_case;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("template").assert();

    res.stderr(contains("Usage: rtf template"));
}

#[test_case(
    "does-not-exist.yaml",
    "No such file or directory (os error 2)";
    "file does not exist"
)]
#[test_case(
    "not-yaml.txt",
    "Failed to parse test plan yaml: invalid type: string \"This is not a yaml file\", expected struct RawTestPlanConfig";
    "file is not yaml"
)]
#[test_case(
    "not-a-test-plan.yaml",
    "Failed to parse test plan yaml: missing field `name`";
    "file not test plan yaml"
)]
#[test_case(
    "missing-required-field.yaml",
    "Failed to parse test plan yaml: missing field `scenario`";
    "test plan missing required fields"
)]
#[test_case(
    "invalid-config-spec.yaml",
    "Failed to parse test plan yaml: expected valid inline config section or from with overrides";
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
