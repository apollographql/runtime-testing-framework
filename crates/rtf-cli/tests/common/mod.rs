use assert_cmd::{Command, cargo::cargo_bin_cmd};
use assert_fs::{
    TempDir,
    fixture::ChildPath,
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

impl CmdWithTmpDir {
    pub fn new(cmd: Command, tmp: TempDir) -> Self {
        Self { cmd, tmp }
    }
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
    pub fn child(&self, path: &str) -> ChildPath {
        self.tmp.child(path)
    }

    pub fn child_path(&self, path: &str) -> PathBuf {
        self.tmp.child(path).to_path_buf()
    }

    /// Assert that a given path within the test [TempDir] exists.
    pub fn assert_path_exists(&self, path: &str) {
        assert!(self.tmp.child(path).exists(), "{path} does not exist")
    }

    /// Assert that a given file within the test [TempDir] does not contain `value`.
    pub fn assert_file_does_not_contain(&self, path: &str, value: &str) {
        let file = self.tmp.child(path);
        let content = std::fs::read_to_string(file.path())
            .unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
        assert!(
            !content.contains(value),
            "file {path} unexpectedly contains: {value}"
        );
    }

    /// Assert that a given file within the test [TempDir] contains `value`.
    pub fn assert_file_contains(&self, path: &str, value: &str) {
        let file = self.tmp.child(path);
        let content = std::fs::read_to_string(file.path())
            .unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
        assert!(
            content.contains(value),
            "file {path} does not contain expected value: {value}"
        );
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

pub struct TestSetup {
    pub tmp: TempDir,
    pub output_file_path: PathBuf,
    pub test_plan_file_path: PathBuf,
}

pub fn prepare_for_test(dir: &str) -> TestSetup {
    // For the sake of tests that need to volume mount into docker containers, we place our temp
    // directories in CARGO_TARGET_TMPDIR rather than /tmp. This allows us to avoid all of the
    // "fun" of OSX /tmp symlinks and the fact that docker under OSX runs in a VM that doesn't have
    // access to paths outside of the user's homedir.
    //   See https://doc.rust-lang.org/cargo/reference/environment-variables.html
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from(dir, &["**"]).unwrap();

    let output_file_path = tmp.child("output").path().to_path_buf();
    let test_plan_file_path = tmp.child("test-plan.yaml").path().to_path_buf();

    TestSetup {
        tmp,
        output_file_path,
        test_plan_file_path,
    }
}

pub fn prepare_rtf_run(dir: &str) -> CmdWithTmpDir {
    let test_setup = prepare_for_test(dir);
    let mut cmd = cargo_bin_cmd!("rtf");

    cmd.arg("run")
        .arg(&test_setup.test_plan_file_path)
        .arg("--outdir")
        .arg(&test_setup.output_file_path)
        .arg("-vv");

    CmdWithTmpDir {
        cmd,
        tmp: test_setup.tmp,
    }
}

pub fn prepare_rtf_remote_prepare(dir: &str) -> CmdWithTmpDir {
    let test_setup = prepare_for_test(dir);
    let mut cmd = cargo_bin_cmd!("rtf");

    cmd.arg("remote")
        .arg("prepare")
        .arg(&test_setup.test_plan_file_path)
        .arg("-vv");

    CmdWithTmpDir {
        cmd,
        tmp: test_setup.tmp,
    }
}

/// Prepare an rtf run command with a variables file containing the given content.
pub fn prepare_rtf_run_with_vars_file(dir: &str, vars_content: &str) -> CmdWithTmpDir {
    use std::fs;

    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from(dir, &["**"]).unwrap();

    // Write the variables file
    let vars_file_path = tmp.child("test-variables.json");
    fs::write(vars_file_path.path(), vars_content).unwrap();

    let output_file_path = tmp.child("output");
    let output_file_path = output_file_path.path().to_str().unwrap();

    let test_plan_file_path = tmp.child("test-plan.yaml");
    let test_plan_file_path = test_plan_file_path.path().to_str().unwrap();

    let vars_file_path_str = vars_file_path.path().to_str().unwrap();

    let mut cmd = cargo_bin_cmd!("rtf");

    cmd.arg("run")
        .arg(test_plan_file_path)
        .arg("--outdir")
        .arg(output_file_path)
        .arg("--vars")
        .arg(vars_file_path_str)
        .arg("-vv");

    CmdWithTmpDir { cmd, tmp }
}
