//! Runtime Testing Framework CLI - a swiss army knife for testing the Apollo Runtime
use rtf_core::variables::Variables;

pub mod cli;
pub mod commands;

/// The environment variable to set to control logging within the rtf CLI
pub const LOG_LEVEL_ENV_VAR: &str = "APOLLO_RTF_LOG";

// The `cli.rs` file is pulled in as an inline module so we can generate markdown help for the CLI
// interface in `build.rs`. Any dependencies we make use of in that file need to be included in the
// build dependencies (rather than main crate dependencies), so we implement this method here
// instead.

impl From<cli::Variables> for Variables {
    fn from(v: cli::Variables) -> Self {
        Self {
            var: v.var,
            vars: v.vars,
        }
    }
}
