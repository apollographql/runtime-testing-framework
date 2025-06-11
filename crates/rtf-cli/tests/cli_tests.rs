pub mod common;

use assert_cmd::Command;
use common::is_valid_test_plan;
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
fn run_command_invalid_test_plan_path_errors() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();

    let res = cmd.arg("run").arg("/not/a/file.txt").assert();

    res.stdout(contains("No such file or directory (os error 2)"));
}

#[test]
fn run_command_sanity_check_works() {
    is_valid_test_plan("resources/sanity-check");
}
