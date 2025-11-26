//! Integration tests for custom provider plumbing commands
use assert_cmd::Command;
use assert_fs::{TempDir, prelude::*};
use predicates::str::contains;

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
fn check_completes_simple() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/valid/custom-provider-standalone/simple-provider.yaml")
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
        .arg("resources/valid/custom-provider-standalone/with-variables.yaml")
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
        .arg("resources/valid/custom-provider-standalone/with-default.yaml")
        .arg("--check")
        .assert();

    // Should succeed using the default value
    res.success().stdout(contains("name: "));
}

#[test]
fn check_with_missing_required_variable_fails() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("template")
        .arg("resources/valid/custom-provider-standalone/with-variables.yaml")
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
        .arg("resources/invalid/custom-provider/missing-command.yaml")
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
        .arg("resources/invalid/custom-provider/missing-variable.yaml")
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
        .arg("resources/invalid/custom-provider/missing-file.yaml")
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
        .arg("resources/valid/custom-provider-standalone/with-variables.yaml")
        .arg("--vars")
        .arg("resources/valid/custom-provider-standalone/test-vars-array.json")
        .arg("--check")
        .assert();

    res.failure()
        .stderr(contains("found matrix dimensions for: items"));
}

#[test]
fn run_creates_expected_output() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/valid/custom-provider-standalone",
        &["simple-provider.yaml"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("run")
        .arg(tmp.child("simple-provider.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    assert!(output_dir.exists());

    let output_file = output_dir.child("RTF_OUTPUT");
    assert!(output_file.exists());

    let content = std::fs::read_to_string(output_file.path()).unwrap();
    assert!(content.contains("Hello from simple provider"));
}

#[test]
fn run_with_variables_creates_expected_output() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/valid/custom-provider-standalone",
        &["with-variables.yaml"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("run")
        .arg(tmp.child("with-variables.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("--var")
        .arg("message=HelloWorld")
        .assert();

    res.success();

    let output_file = output_dir.child("RTF_OUTPUT");
    let content = std::fs::read_to_string(output_file.path()).unwrap();
    assert!(content.contains("HelloWorld"));
}

#[test]
fn run_with_existing_outdir_fails() {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(
        "resources/valid/custom-provider-standalone",
        &["simple-provider.yaml"],
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
        .arg(tmp.child("simple-provider.yaml").path())
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
        "resources/valid/custom-provider-standalone",
        &["with-file-provider.yaml", "scripts/**"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd
        .env_clear()
        .arg("custom-provider")
        .arg("run")
        .arg(tmp.child("with-file-provider.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.success();

    let output_file = output_dir.child("RTF_OUTPUT");
    let content = std::fs::read_to_string(output_file.path()).unwrap();
    assert!(content.contains("This is test input"));
}
