//! Integration tests for resolve plumbing commands
use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::{TempDir, prelude::*};
use predicates::str::contains;
use std::fs;

#[test]
fn resolve_is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd.arg("resolve").assert();

    res.failure()
        .stderr(contains("Resolve file providers for a config file"));
}

#[test]
fn resolve_scenario_is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd.arg("resolve").arg("scenario").assert();

    res.failure()
        .stderr(contains("required arguments were not provided"));
}

#[test]
fn resolve_script_scenario_creates_expected_output() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/valid/script-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    // Check scenario.env has expected content
    let env_content = fs::read_to_string(output_dir.child("scenario.env").path()).unwrap();
    let out = output_dir.path().to_str().unwrap();
    let expected = r#"export DATA_FILE="{OUT}/providers/data.txt"
export MY_VAR="my_value"
export OUTDIR="{OUT}"
export RTF_OUTPUT="{OUT}/RTF_OUTPUT"
"#
    .replace("{OUT}", out);

    assert_eq!(env_content, expected);

    // Check provider file was written
    let provider_content =
        fs::read_to_string(output_dir.child("providers/data.txt").path()).unwrap();
    assert_eq!(provider_content, "test data content");
}

#[test]
fn resolve_docker_scenario_creates_expected_output() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/valid/docker-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    // Check scenario.env has expected content with HOST paths (not container /output/... paths)
    let env_content = fs::read_to_string(output_dir.child("scenario.env").path()).unwrap();
    let out = output_dir.path().to_str().unwrap();
    let expected = r#"export DATA_FILE="{OUT}/providers/data.txt"
export DOCKER_VAR="docker_value"
export OUTDIR="{OUT}"
export RTF_OUTPUT="{OUT}/RTF_OUTPUT"
"#
    .replace("{OUT}", out);

    assert_eq!(env_content, expected);

    // Check provider file was written
    assert!(output_dir.child("providers/data.txt").exists());
}

#[test]
fn resolve_scenario_with_custom_providers_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/invalid/with-custom-providers", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("custom_providers are not supported"));
}

#[test]
fn resolve_scenario_with_missing_variables_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/invalid/missing-variables", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("Templating failed"))
        .stderr(contains("Unknown templating variable"))
        .stderr(contains("undefined_var"));
}
