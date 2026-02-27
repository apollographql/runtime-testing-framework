use super::{
    CUSTOM_PROVIDER, GITHUB_FILE, MERGE_YAML, RELATIVE_PATH, prepare_rtf_inline_all,
    prepare_rtf_inline_all_from_file,
};
use assert_fs::prelude::*;
use predicates::str::contains;
use simple_test_case::test_case;

#[test]
fn with_cli_variables_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/sanity-check");
    cmd.arg("--var").arg("setup_output=\"setup output\"");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
    // After inline all, github_file should also be inlined
    cmd.assert_file_does_not_contain(inlined_output_path, GITHUB_FILE);
}

#[test]
fn matrix_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/matrix-variables");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_paths = [
        "output/matrix_variant_1-inlined-test-plan.yaml",
        "output/matrix_variant_2-inlined-test-plan.yaml",
        "output/matrix_variant_3-inlined-test-plan.yaml",
        "output/matrix_variant_4-inlined-test-plan.yaml",
    ];

    for path in inlined_output_paths {
        cmd.assert_path_exists(path);
        cmd.assert_file_does_not_contain(path, RELATIVE_PATH);
    }
}

#[test]
fn matrix_custom_variant_names_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/custom-matrix-variant-names");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_paths = [
        "output/world!-mother-inlined-test-plan.yaml",
        "output/world!-father-inlined-test-plan.yaml",
        "output/sailor-mother-inlined-test-plan.yaml",
        "output/sailor-father-inlined-test-plan.yaml",
    ];

    for path in inlined_output_paths {
        cmd.assert_path_exists(path);
        cmd.assert_file_does_not_contain(path, RELATIVE_PATH);
    }
}

#[test]
fn matrix_include_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/matrix-include");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_paths = [
        "output/matrix_variant_1-inlined-test-plan.yaml",
        "output/matrix_variant_2-inlined-test-plan.yaml",
        "output/matrix_variant_3-inlined-test-plan.yaml",
        "output/matrix_variant_4-inlined-test-plan.yaml",
    ];

    for path in inlined_output_paths {
        cmd.assert_path_exists(path);
        cmd.assert_file_does_not_contain(path, RELATIVE_PATH);
    }
}

#[test]
fn relative_path_from_template_variable_succeeds() {
    let mut cmd = prepare_rtf_inline_all(
        "resources/test-plans/valid/regression-relative-path-from-template-variable",
    );
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
}

#[test]
fn command_from_spec_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/command-from-spec");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
}

#[test]
fn custom_provider_default_value_succeeds() {
    let mut cmd =
        prepare_rtf_inline_all("resources/test-plans/valid/custom-provider-default-value");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(inlined_output_path, CUSTOM_PROVIDER);
}

#[test]
fn custom_provider_templated_variable_succeeds() {
    let mut cmd =
        prepare_rtf_inline_all("resources/test-plans/valid/custom-provider-templated-variable");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(inlined_output_path, CUSTOM_PROVIDER);
}

#[test]
fn custom_provider_static_argument_succeeds() {
    let mut cmd =
        prepare_rtf_inline_all("resources/test-plans/valid/custom-provider-static-argument");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(inlined_output_path, CUSTOM_PROVIDER);
}

#[test]
fn all_sections_inlined_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/all-sections-relative-paths");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
    // merge_yaml should be converted to inline (unlike relative-files which keeps it)
    cmd.assert_file_does_not_contain(inlined_output_path, MERGE_YAML);
}

#[test]
fn from_command_provider_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/from-command-provider-dir");
    cmd.assert().success();

    cmd.list_files();

    let inlined_output_path = "output/inlined-test-plan.yaml";

    cmd.assert_path_exists(inlined_output_path);
    // from_command uses inline_in_place - it inlines nested providers but keeps its structure
    // since it needs to execute a command at runtime
    cmd.assert_file_does_not_contain(inlined_output_path, RELATIVE_PATH);
}

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
fn execution_fails_when_load_and_resolve_fails(file: &str, err_contains: &str) {
    let mut cmd = prepare_rtf_inline_all_from_file(&format!(
        "resources/test-plans/invalid/load-and-resolve/{file}"
    ));
    let res = cmd.env_clear().assert();

    res.failure().stderr(contains(err_contains));
}

#[test]
fn with_existing_outdir_fails() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/command-from-spec");
    let out_dir = cmd.child("output");
    out_dir.create_dir_all().unwrap();
    out_dir.child("existing.txt").write_str("content").unwrap();

    cmd.assert()
        .failure()
        .stderr(contains("already exists and is non-empty"));
}

#[test]
fn with_existing_outdir_and_force_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/command-from-spec");
    let out_dir = cmd.child("output");
    out_dir.create_dir_all().unwrap();
    out_dir.child("existing.txt").write_str("content").unwrap();

    cmd.arg("--force").assert().success();
}
