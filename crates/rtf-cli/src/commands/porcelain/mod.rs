//! High level user commands for running the most common workflows and functionality exposed
//! through the rtf CLI.

mod run;

pub use run::{check_and_run_github_test_plan, check_and_run_local_test_plan};
