use assert_cmd::{Command, cargo::cargo_bin_cmd};
use assert_fs::{
    TempDir,
    prelude::{PathChild, PathCopy},
};
use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
};
use walkdir::WalkDir;

/// [TempDir] removes the temp directory it creates on drop so we need to bundle it with the
/// [Command] we want to execute in order to keep things in place for the duration of the test.
pub struct CmdWithTmpDir {
    cmd: Command,
    tmp: TempDir,
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

impl CmdWithTmpDir {
    pub fn child_path(&self, path: &str) -> PathBuf {
        self.tmp.child(path).to_path_buf()
    }

    /// Assert that a given path within the test [TempDir] exists.
    pub fn assert_path_exists(&self, path: &str) {
        assert!(self.tmp.child(path).exists(), "{path} does not exist")
    }

    /// Debugging helper for showing what the contents of this test's temp directory were.
    pub fn list_files(&self) {
        println!(">> Temp directory contents:");
        for entry in WalkDir::new(self.tmp.path()) {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                let p = entry.path().strip_prefix(self.tmp.path()).unwrap();
                println!("{}", p.display());
            }
        }
    }
}

/// Assert that a given test-plan is valid.
///
/// Test plans must be self contained with all associated files under the specified directory
pub fn is_valid_test_plan(dir: &str) {
    prepare_rtf_run(dir).assert().success();
}

pub fn prepare_rtf_run(dir: &str) -> CmdWithTmpDir {
    let tmp = TempDir::new().unwrap();
    tmp.copy_from(dir, &["**"]).unwrap();

    let output_file_path = tmp.child("output");
    let output_file_path = output_file_path.path().to_str().unwrap();

    let test_plan_file_path = tmp.child("test-plan.yaml");
    let test_plan_file_path = test_plan_file_path.path().to_str().unwrap();

    let mut cmd = cargo_bin_cmd!("rtf");

    cmd.arg("run")
        .arg(test_plan_file_path)
        .arg("--outdir")
        .arg(output_file_path)
        .arg("-vv");

    CmdWithTmpDir { cmd, tmp }
}
