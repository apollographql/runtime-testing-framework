use crate::inline::{
    GITHUB_FILE, RELATIVE_PATH, prepare_rtf_inline_all, prepare_rtf_inline_relative_files,
};
use predicates::str::contains;

const INLINED_OUTPUT_PATH: &str = "output/inlined-test-plan.yaml";

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn relative_files_with_github_file_provider_succeeds() {
    let mut cmd = prepare_rtf_inline_relative_files("resources/test-plans/valid/github-file");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    // github_file should still be present since we only inline relative_path
    cmd.assert_file_contains(INLINED_OUTPUT_PATH, GITHUB_FILE);
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn all_with_github_file_provider_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/github-file");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, GITHUB_FILE);
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn all_with_github_flag_succeeds() {
    use assert_cmd::cargo::cargo_bin_cmd;
    use assert_fs::TempDir;

    let tmp = TempDir::new().unwrap();
    let outdir = tmp.path().join("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    cmd.arg("inline")
        .arg("all")
        .arg("--github")
        .arg(
            "apollographql/runtime-testing-framework/example-test-plans/hello-world/test-plan.yaml",
        )
        .arg("--outdir")
        .arg(&outdir)
        .arg("-vv")
        .assert()
        .success()
        .stderr(contains("done"));
}

#[test]
#[ignore = "requires a valid GitHub API Token"]
fn relative_files_with_github_flag_succeeds() {
    use assert_cmd::cargo::cargo_bin_cmd;
    use assert_fs::TempDir;

    let tmp = TempDir::new().unwrap();
    let outdir = tmp.path().join("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    cmd.arg("inline")
        .arg("relative-files")
        .arg("--github")
        .arg(
            "apollographql/runtime-testing-framework/example-test-plans/hello-world/test-plan.yaml",
        )
        .arg("--outdir")
        .arg(&outdir)
        .arg("-vv")
        .assert()
        .success()
        .stderr(contains("done"));
}
