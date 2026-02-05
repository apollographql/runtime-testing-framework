use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
fn is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");

    let res = cmd.arg("json-schemas").assert();

    res.stderr(contains(
        "the following required arguments were not provided:",
    ));
}

#[test]
fn schemas_test_plan_writes_json_schema_to_stdout() {
    let mut cmd = cargo_bin_cmd!("rtf");

    let res = cmd.args(["json-schemas", "test-plan"]).assert();

    res.success()
        .stdout(contains(
            r#""$schema": "http://json-schema.org/draft-07/schema#""#,
        ))
        .stdout(contains(r#""title": "Test Plan Config""#));
}

#[test]
fn schemas_environment_writes_json_schema_to_stdout() {
    let mut cmd = cargo_bin_cmd!("rtf");

    let res = cmd.args(["json-schemas", "environment"]).assert();

    res.success()
        .stdout(contains(
            r#""$schema": "http://json-schema.org/draft-07/schema#""#,
        ))
        .stdout(contains(r#""title": "Environment Config""#));
}

#[test]
fn schemas_scenario_writes_json_schema_to_stdout() {
    let mut cmd = cargo_bin_cmd!("rtf");

    let res = cmd.args(["json-schemas", "scenario"]).assert();

    res.success()
        .stdout(contains(
            r#""$schema": "http://json-schema.org/draft-07/schema#""#,
        ))
        .stdout(contains(r#""title": "Scenario Config""#));
}
