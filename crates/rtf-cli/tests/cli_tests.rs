use assert_cmd::Command;
use assert_fs::{
    TempDir,
    prelude::{PathChild, PathCopy},
};
use predicates::str::contains;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    // Running with no args should return a help message to std_err
    let res = cmd.assert();

    // Check the output contains usage instructions for rtf
    res.stderr(contains("Usage: rtf"));
}

#[test]
fn run_command_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").assert();

    // Check the output contains usage instructions for rtf run
    res.stderr(contains("Usage: rtf run"));
}

#[test]
fn run_command_sanity_check_works() {
    let temp = TempDir::new().unwrap();
    temp.copy_from("resources/sanity-check", &["**"]).unwrap();

    for entry in temp.path().read_dir().expect("read_dir call failed") {
        if let Ok(entry) = entry {
            println!("{:?}", entry.path());
        }
    }

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

#[test]
fn run_command_invalid_test_plan_path_errors() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").arg("/not/a/file.txt").assert();

    res.stdout(contains("No such file or directory (os error 2)"));
}
