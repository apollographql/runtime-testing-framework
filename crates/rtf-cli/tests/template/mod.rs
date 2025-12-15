use assert_cmd::cargo::cargo_bin_cmd;
use indoc::indoc;
use predicates::str::{contains, is_match};
use simple_test_case::test_case;

#[test]
fn is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd.arg("template").assert();

    res.stderr(contains("no test plan provided"));
}

#[test_case("backwards-compatible-variable-config"; "backwards compatible variable config")]
#[test_case("command-from-spec"; "command from spec")]
#[test_case("custom-matrix-variant-names"; "custom matrix variant names")]
#[test_case("custom-provider-default-value"; "custom provider default value")]
#[test_case("custom-provider-static-argument"; "custom provider static argument")]
#[test_case("custom-provider-templated-variable"; "custom provider templated variable")]
#[test_case("matrix-include"; "matrix include")]
#[test_case("matrix-variables"; "matrix variables")]
#[test_case("variable-overrides"; "variable overrides")]
// Template and check all valid test plans except for the sanity check (which requires a provides variable)
// and the github and graphos test plans which are tested in their respective modules
#[test]
fn check_completes_basic(test_plan_dir: &str) {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!(
            "resources/test-plans/valid/{test_plan_dir}/test-plan.yaml"
        ))
        .arg("--check")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test]
fn check_completes_with_cli_variables() {
    // The sanity-check test plan defines variables in the setup.provides
    // The only way to template successfully is to set this variable from the cli
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("resources/test-plans/valid/sanity-check/test-plan.yaml")
        .arg("--check")
        .arg("--var")
        .arg("setup_output=\"setup output\"")
        .assert();

    // Check that a test plan gets printed to stdout
    res.success().stdout(contains("name:"));
}

#[test]
fn check_completes_with_backwards_compatible_cli_variables_flag() {
    // The sanity-check test plan defines variables in the setup.provides
    // The only way to template successfully is to set this variable from the cli
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("resources/test-plans/valid/sanity-check/test-plan.yaml")
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
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!(
            "resources/test-plans/invalid/load-and-resolve/{file}"
        ))
        .assert();

    res.stderr(contains(err_contains));
}

#[test_case(
    "conflicting-keys.yaml",
    "(test_plan) Conflicting variable and matrix definitions\nfoo";
    "conflicting keys"
)]
#[test_case(
    "empty-matrix.yaml",
    "(test_plan) Empty array for matrix variable\nfoo";
    "empty matrix"
)]
#[test_case(
    "inconsistent-matrix-variables.yaml",
    "(test_plan) Inconsistent types for matrix variable\nfoo";
    "inconsistent matrix variables"
)]
#[test_case(
    "inconsistent-matrix-include.yaml",
    "(test_plan) Inconsistent types for matrix include maps\nmatrix include maps must share consistent keys and types";
    "inconsistent matrix include"
)]
#[test_case(
    "missing-variables.yaml",
    indoc!(r#"
    (environment.setup.env_vars.BAR) Missing template variables definition. Make sure the variable is defined in the scenario or environment config variable definitions
      - bar: "BAR"
    
    (environment.teardown.env_vars.BAZ) Missing template variables definition. Make sure the variable is defined in the scenario or environment config variable definitions
      - baz: "BAZ"
    
    (scenario.env_vars.FOO) Missing template variables definition. Make sure the variable is defined in the scenario or environment config variable definitions
      - foo: "FOO"
    "#);
    "missing variables"
)]
#[test_case(
    "unknown-variables.yaml",
    "(environment.teardown.env_vars.FOO) Unknown templating variable. Make sure a value is defined for this variable to resolve to.\nfoo";
    "unknown variables"
)]
#[test]
fn templating_fails(file: &str, err_contains: &str) {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!("resources/test-plans/invalid/templating/{file}"))
        .assert();

    res.stderr(contains(format!("Templating failed\n{err_contains}")));
}

#[test]
fn duplicate_variant_names_fails() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .arg("template")
        .arg("resources/test-plans/invalid/templating/duplicate-variant-names.yaml")
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
    "duplicate-variable-names.yaml",
    "(scenario.variables) Non-unique variable names found\nfoo";
    "duplicate variable names"
)]
#[test_case(
    "duplicate-env-vars.yaml",
    "(scenario) Non-unique environment variables found\nFOO";
    "duplicate env vars"
)]
#[test]
fn check_fails(file: &str, err_contains: &str) {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg(format!("resources/test-plans/invalid/checks/{file}"))
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
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("resources/test-plans/invalid/checks/missing-relative-file.yaml")
        .arg("--check")
        .assert();

    res.stderr(
        is_match(
            r#"Static analysis checks failed
\(scenario\.command\.command_provider\) The requested file did not exist
provided path was file://.*/resources/test-plans/invalid/checks/does-not-exist\.sh"#,
        )
        .unwrap(),
    );
}

#[test]
fn load_and_resolve_from_invalid_github_uri_fails() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("--github")
        .arg("not a valid github uri")
        .assert();

    res.failure().stderr(contains("invalid GitHub uri: \"not a valid github uri\" - GitHub uri must be in format ORG/REPO/PATH"));
}

#[test]
fn load_and_resolve_from_github_missing_token_fails() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear() // Clear the environment to ensure no keys have been provided
        .arg("template")
        .arg("--github")
        .arg("org/repo/path")
        .assert();

    res.failure().stderr(contains("no GitHub client available"));
}
