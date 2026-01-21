mod all;
mod relative_files;

use crate::common::{CmdWithTmpDir, prepare_for_test};
use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::{TempDir, prelude::PathChild};
use predicates::str::contains;
use std::fs::copy;

// Markers for provider kinds that should be inlined
pub const RELATIVE_PATH: &str = "kind: relative_path";
pub const GITHUB_FILE: &str = "kind: github_file";
pub const FROM_COMMAND: &str = "kind: from_command";
pub const CUSTOM_PROVIDER: &str = "kind: custom_provider";
pub const MERGE_YAML: &str = "kind: merge_yaml";

// GraphOS provider markers
pub const GRAPHOS_SUPERGRAPH: &str = "kind: graphos_supergraph";
pub const GRAPHOS_SUBGRAPHS: &str = "kind: graphos_subgraphs";
pub const GRAPHOS_SUBGRAPH_NAMES: &str = "kind: graphos_subgraph_names";
pub const GRAPHOS_SUBGRAPH_ROUTER_URL_OVERRIDES: &str =
    "kind: graphos_subgraph_router_url_overrides";
pub const GRAPHOS_CANNED_OPS: &str = "kind: graphos_canned_ops";
pub const GRAPHOS_CANNED_OPS_BY_ID: &str = "kind: graphos_canned_ops_by_id";
pub const OFFLINE_GRAPHOS_LICENSE: &str = "kind: offline_graphos_license";

// The only kind that should remain after inlining
pub const INLINE: &str = "kind: inline";

pub fn prepare_rtf_inline_relative_files(dir: &str) -> CmdWithTmpDir {
    prepare_rtf_inline("relative-files", dir)
}

pub fn prepare_rtf_inline_all(dir: &str) -> CmdWithTmpDir {
    prepare_rtf_inline("all", dir)
}

fn prepare_rtf_inline(subcommand: &str, dir: &str) -> CmdWithTmpDir {
    let test_setup = prepare_for_test(dir);
    let mut cmd = cargo_bin_cmd!("rtf");

    cmd.arg("inline")
        .arg(subcommand)
        .arg(&test_setup.test_plan_file_path)
        .arg("--outdir")
        .arg(&test_setup.output_file_path)
        .arg("-vv");

    CmdWithTmpDir::new(cmd, test_setup.tmp)
}

/// Prepare an rtf inline command when given a single test plan file path.
/// The file will be copied into a temporary directory as `test-plan.yaml`
pub fn prepare_rtf_inline_relative_files_from_file(file_path: &str) -> CmdWithTmpDir {
    prepare_rtf_inline_from_file("relative-files", file_path)
}

pub fn prepare_rtf_inline_all_from_file(file_path: &str) -> CmdWithTmpDir {
    prepare_rtf_inline_from_file("all", file_path)
}

fn prepare_rtf_inline_from_file(subcommand: &str, file_path: &str) -> CmdWithTmpDir {
    let tmp_src = TempDir::new().unwrap();
    let dest = tmp_src.child("test-plan.yaml");
    copy(file_path, dest.path()).unwrap();

    prepare_rtf_inline(subcommand, tmp_src.path().to_str().unwrap())
}

#[test]
fn is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd.arg("inline").assert();

    res.stderr(contains("Usage: rtf inline [OPTIONS] <COMMAND>"));
}
