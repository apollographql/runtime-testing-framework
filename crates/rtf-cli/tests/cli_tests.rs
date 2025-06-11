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

// FIXME: move these to files that are named per API key / graph that's required

// This test uses the imgood-observability-test graph, current variant in the apollo-team-runtime-readiness
// studio org to run. You need to create an API Key with Graph Admin permissions for this graph to successfully
// complete the test. The API key should be set in the command for running the test e.g.
// APOLLO_KEY=<YOUR_KEY_HERE> cargo test --test cli_tests
#[test]
#[ignore = "requires a valid GraphOS API Key"]
fn run_command_graphos_supergraph_works() {
    let temp = TempDir::new().unwrap();
    temp.copy_from("resources/graphos-supergraph", &["**"])
        .unwrap();

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

// This test uses the imgood-observability-test graph, current variant in the apollo-team-runtime-readiness
// studio org to run. You need to create an API Key with Graph Admin permissions for this graph to successfully
// complete the test. The API key should be set in the command for running the test e.g.
// GRAPHOS_API_KEY=<YOUR_KEY_HERE> cargo test --test cli_tests
#[test]
#[ignore = "requires a valid GraphOS API Key"]
fn run_command_graphos_canned_ops_works() {
    let temp = TempDir::new().unwrap();
    temp.copy_from("resources/graphos-canned-ops", &["**"])
        .unwrap();

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

// This test uses the starstuff graph.
// You need to create an API Key with Graph Admin permissions for this graph to successfully
// complete the test. The API key should be set in the command for running the test e.g.
// APOLLO_KEY=<YOUR_KEY_HERE> cargo test --test cli_tests
#[test]
#[ignore = "requires a valid GraphOS API Key"]
fn run_command_offline_graphos_license_works() {
    let temp = TempDir::new().unwrap();
    temp.copy_from("resources/graphos-offline-license", &["**"])
        .unwrap();

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
