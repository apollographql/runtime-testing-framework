use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("template").assert();

    res.stderr(contains("Usage: rtf template"));
}

#[test]
fn invalid_test_plan_path_errors() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    let res = cmd.arg("template").arg("/not/a/file.txt").assert();

    res.stderr(contains("No such file or directory (os error 2)"));
}
