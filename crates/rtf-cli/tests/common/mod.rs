use assert_cmd::Command;
use assert_fs::{
    TempDir,
    prelude::{PathChild, PathCopy},
};

/// Assert that a given test-plan is valid.
///
/// Test plans must be self contained with all associated files under the specified directory
pub fn is_valid_test_plan(dir: &str) {
    let temp = TempDir::new().unwrap();
    temp.copy_from(dir, &["**"]).unwrap();

    let output_file_path = temp.child("output");
    let output_file_path = output_file_path.path().to_str().unwrap();

    let test_plan_file_path = temp.child("test-plan.yaml");
    let test_plan_file_path = test_plan_file_path.path().to_str().unwrap();

    let mut cmd = Command::cargo_bin("rtf").unwrap();

    cmd.arg("run")
        .arg(test_plan_file_path)
        .arg("--outdir")
        .arg(output_file_path)
        .assert()
        .success();
}
