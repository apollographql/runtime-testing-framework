use std::ops::{Deref, DerefMut};

use assert_cmd::Command;
use assert_fs::{
    TempDir,
    prelude::{PathChild, PathCopy},
};

/// [TempDir] removes the temp directory it creates on drop so we need to bundle it with the
/// [Command] we want to execute in order to keep things in place for the duration of the test.
pub struct CmdWithTmpDir {
    cmd: Command,
    _tmp: TempDir,
}

impl Deref for CmdWithTmpDir {
    type Target = Command;

    fn deref(&self) -> &Self::Target {
        &self.cmd
    }
}

impl DerefMut for CmdWithTmpDir {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cmd
    }
}

/// Assert that a given test-plan is valid.
///
/// Test plans must be self contained with all associated files under the specified directory
pub fn is_valid_test_plan(dir: &str) {
    prepare_rtf_run(dir).assert().success();
}

pub fn prepare_rtf_run(dir: &str) -> CmdWithTmpDir {
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
        .arg(output_file_path);

    CmdWithTmpDir { cmd, _tmp: temp }
}
