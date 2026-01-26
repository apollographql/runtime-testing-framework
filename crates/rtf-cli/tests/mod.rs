pub mod common;
pub mod custom_provider;
pub mod docker;
pub mod expand_matrix;
pub mod github;
pub mod graphos;
pub mod inline;
pub mod run;
pub mod template;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
fn rtf_is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    // Running with no args should return a help message to std_err
    let res = cmd.assert();

    // Check the output contains usage instructions for rtf
    res.stderr(contains("Usage: rtf"));
}
