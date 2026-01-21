use crate::inline::{
    OFFLINE_GRAPHOS_LICENSE, RELATIVE_PATH, prepare_rtf_inline_all,
    prepare_rtf_inline_relative_files,
};

const INLINED_OUTPUT_PATH: &str = "output/inlined-test-plan.yaml";

#[test]
#[ignore = "requires a valid GraphOS API Key for the starstuff graph"]
fn relative_files_with_offline_license_succeeds() {
    let mut cmd =
        prepare_rtf_inline_relative_files("resources/test-plans/valid/graphos-offline-license");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    // offline_graphos_license should still be present since we only inline relative_path
    cmd.assert_file_contains(INLINED_OUTPUT_PATH, OFFLINE_GRAPHOS_LICENSE);
}

#[test]
#[ignore = "requires a valid GraphOS API Key for the starstuff graph"]
fn all_with_offline_graphos_license_succeeds() {
    let mut cmd = prepare_rtf_inline_all("resources/test-plans/valid/graphos-offline-license");
    cmd.assert().success();

    cmd.list_files();

    cmd.assert_path_exists(INLINED_OUTPUT_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, RELATIVE_PATH);
    cmd.assert_file_does_not_contain(INLINED_OUTPUT_PATH, OFFLINE_GRAPHOS_LICENSE);
}
