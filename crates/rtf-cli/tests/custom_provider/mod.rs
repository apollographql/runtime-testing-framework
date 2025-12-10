//! Integration tests for custom provider plumbing commands
use assert_cmd::Command;
use assert_fs::{TempDir, prelude::*};
use predicates::str::contains;
use std::fs;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("custom-provider").assert();

    res.failure().stderr(contains(
        "Work directly with custom file provider definitions",
    ));
}

#[test]
fn template_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("custom-provider").arg("template").assert();

    res.failure()
        .stderr(contains("required arguments were not provided"));
}

#[test]
fn run_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("custom-provider").arg("run").assert();

    res.failure()
        .stderr(contains("required arguments were not provided"));
}

#[test]
fn test_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("custom-provider").arg("test").assert();

    res.failure()
        .stderr(contains("required arguments were not provided"));
}

#[test]
fn check_completes_simple() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/valid/single-file/provider.yaml")
        .arg("--check")
        .assert();

    res.success().stdout(contains("name: "));
}

#[test]
fn check_completes_with_variables() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/valid/with-variables/provider.yaml")
        .arg("--var")
        .arg("message=test")
        .arg("--check")
        .assert();

    res.success().stdout(contains("name: "));
}

#[test]
fn check_completes_with_default_variable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/valid/with-default-variable/provider.yaml")
        .arg("--check")
        .assert();

    res.success().stdout(contains("name: "));
}

#[test]
fn check_with_missing_required_variable_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/valid/with-variables/provider.yaml")
        .arg("--check")
        .assert();

    res.failure()
        .stderr(contains("Templating failed"))
        .stderr(contains("Unknown templating variable"))
        .stderr(contains("message"));
}

#[test]
fn check_with_invalid_yaml_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/invalid/missing-command.yaml")
        .arg("--check")
        .assert();

    res.failure()
        .stderr(contains("Unable to parse custom provider definition yaml"));
}

#[test]
fn check_with_undefined_variable_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/invalid/missing-variable.yaml")
        .arg("--check")
        .assert();

    res.failure()
        .stderr(contains("Templating failed"))
        .stderr(contains("Unknown templating variable"))
        .stderr(contains("undefined_variable"));
}

#[test]
fn check_with_missing_file_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/invalid/missing-file.yaml")
        .arg("--check")
        .assert();

    res.failure()
        .stderr(contains("Static analysis checks failed"))
        .stderr(contains("file did not exist"))
        .stderr(contains("does-not-exist.sh"));
}

#[test]
fn check_with_array_variables_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/custom-providers/valid/with-variables/provider.yaml")
        .arg("--vars")
        .arg("resources/custom-providers/invalid/test-vars-array.json")
        .arg("--check")
        .assert();

    res.failure()
        .stderr(contains("found matrix dimensions for: items"));
}

#[test]
fn run_single_file_creates_expected_output() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/single-file",
        &["provider.yaml"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("run")
        .arg(tmp.child("provider.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    assert!(output_dir.exists());

    let output_file = output_dir.child("RTF_OUTPUT");
    assert!(output_file.exists());

    let content = fs::read_to_string(output_file.path()).unwrap();
    assert!(content.contains("pass"));
}

#[test]
fn run_with_variables_creates_expected_output() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/with-variables",
        &["provider.yaml"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("run")
        .arg(tmp.child("provider.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("--var")
        .arg("message=HelloWorld")
        .assert();

    res.success();

    let output_file = output_dir.child("RTF_OUTPUT");
    let content = fs::read_to_string(output_file.path()).unwrap();
    assert!(content.contains("HelloWorld"));
}

#[test]
fn run_with_existing_outdir_fails() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/single-file",
        &["provider.yaml"],
    )
    .unwrap();

    let output_dir = tmp.child("output");
    output_dir.create_dir_all().unwrap();
    output_dir
        .child("existing-file.txt")
        .write_str("content")
        .unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("run")
        .arg(tmp.child("provider.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("already exists and is non-empty"));
}

#[test]
fn run_with_file_provider_creates_expected_output() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/with-file-provider",
        &["provider.yaml", "data/**"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("run")
        .arg(tmp.child("provider.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.success();

    let output_file = output_dir.child("RTF_OUTPUT");
    let content = fs::read_to_string(output_file.path()).unwrap();
    assert!(content.contains("This is test input"));
}

#[test]
fn test_passing_single_file() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/single-file",
        &["provider.yaml", "test-cases/**"],
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("test")
        .arg(tmp.child("provider.yaml").path())
        .arg("--test-cases-dir")
        .arg(tmp.child("test-cases").path())
        .assert();

    res.success().stdout(contains("PASS"));
}

#[test]
fn test_passing_multi_file() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/multi-file",
        &["provider.yaml", "test-cases/**", "data/**"],
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("test")
        .arg(tmp.child("provider.yaml").path())
        .arg("--test-cases-dir")
        .arg(tmp.child("test-cases").path())
        .assert();

    res.success().stdout(contains("PASS"));
}

#[test]
fn test_passing_expected_error() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/expected-error",
        &["provider.yaml", "test-cases/**"],
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("test")
        .arg(tmp.child("provider.yaml").path())
        .arg("--test-cases-dir")
        .arg(tmp.child("test-cases").path())
        .assert();

    res.success().stdout(contains("PASS"));
}

#[test]
fn test_expected_failure_mismatch() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/invalid/test-error-mismatch",
        &["provider.yaml", "test-cases/**"],
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("test")
        .arg(tmp.child("provider.yaml").path())
        .arg("--test-cases-dir")
        .arg(tmp.child("test-cases").path())
        .assert();

    res.failure()
        .stdout(contains("FAIL"))
        .stdout(contains("wrong error output"));
}

#[test]
fn test_error_on_empty() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/single-file",
        &["provider.yaml"], // not copying in the test case
    )
    .unwrap();
    tmp.child("empty-test-cases").create_dir_all().unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("test")
        .arg(tmp.child("provider.yaml").path())
        .arg("--test-cases-dir")
        .arg(tmp.child("empty-test-cases").path())
        .arg("--error-on-empty")
        .assert();

    res.failure().stderr(contains("No test cases found"));
}

#[test]
fn test_expected_failure_but_provider_passes() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/custom-providers/valid/single-file",
        &["provider.yaml"], // not copying in the test case
    )
    .unwrap();

    // Create test case that expects failure but uses a passing provider
    tmp.child("test-cases/case/variables.json")
        .write_str("{}")
        .unwrap();
    tmp.child("test-cases/case/expected-run-error.txt")
        .write_str("stderr: some expected error")
        .unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("test")
        .arg(tmp.child("provider.yaml").path())
        .arg("--test-cases-dir")
        .arg(tmp.child("test-cases").path())
        .assert();

    res.failure()
        .stdout(contains("FAIL"))
        .stdout(contains("output mismatch"))
        .stdout(contains("unexpected"))
        .stderr(contains("Expected-failure case passed"));
}
