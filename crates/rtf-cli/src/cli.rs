//! Parsing of our command line arguments using Clap's derive API
use clap::{Parser, Subcommand};

// NOTE: All of the doc comments here are parsed by Clap and used to build out the documentation
// seen in the CLI. We treat them as user facing and aim to provide as much useful information as
// possible without overwhelming the user with output when they run '-h' or '--help'.
//
// This file is also pulled in to the `build.rs` of this crate so Args can be passed to
// `clap_markdown` in order to generate the `help.md` file in the root of the repo:
//   -> see https://crates.io/crates/clap-markdown for details

/// A swiss army knife for testing the Apollo Runtime.
///
/// You can set the `RUST_LOG` environment variable to alter the logging output of this tool.
#[derive(Debug, Parser)]
#[clap(about, name = "rtf", long_about = None)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // Porcelain commands
    /// Validate and run a test plan
    Run {
        /// Relative path to the test-plan.yaml file that should be executed
        test_plan_path: String,
        /// Output directory for providers when they run
        #[arg(long, default_value = "output")]
        outdir: String,
    },

    // Plumbing commands
    /// Resolve a test plan using provided values, outputting the resulting config to stdout
    Resolve {
        /// Relative path to the test-plan.yaml file that should be resolve
        test_plan_path: String,
        /// Additional values to use while templating, specified as a JSON object
        #[arg(long)]
        values: Option<String>,
    },
}
