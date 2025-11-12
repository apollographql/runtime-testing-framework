pub mod common;
pub mod expand_matrix;
pub mod github;
pub mod graphos;
pub mod run;
pub mod template;

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn rtf_is_executable() {
    let mut cmd = Command::cargo_bin("rtf").unwrap();
    // Running with no args should return a help message to std_err
    let res = cmd.assert();

    // Check the output contains usage instructions for rtf
    res.stderr(contains("Usage: rtf"));
}
