use crate::inline::{RELATIVE_PATH, prepare_rtf_inline_all, prepare_rtf_inline_relative_files};

const INLINED_OUTPUT_PATH: &str = "output/inlined-test-plan.yaml";

#[test]
fn docker_scenario_inline_succeeds() {
    let mut cmd = prepare_rtf_inline_relative_files("resources/test-plans/valid/docker-scenario");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
}

#[test]
fn docker_scenario_inline_all_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/docker-scenario");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
}
