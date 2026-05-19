//! High level user commands for running the most common workflows and functionality exposed
//! through the rtf CLI.

mod ci_run;
mod docs;
mod run;

pub use ci_run::ci_run;
pub use docs::open_docs;
pub use run::check_and_run_test_plan;
