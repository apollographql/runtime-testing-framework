//! Parsing of our command line arguments using Clap's derive API
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    #[command(flatten)]
    pub values: Values,
}

#[derive(Debug, clap::Args)]
pub struct Values {
    /// A single additional templationg value in the form "key=value"
    #[arg(long, global = true)]
    pub value: Vec<String>,
    /// Path to a JSON file containing additional template values
    #[arg(long, global = true)]
    pub values: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // Porcelain commands
    /// Check and run a test plan
    Run {
        /// Relative path to the test-plan.yaml file that should be executed
        test_plan_path: String,
        /// Output directory for providers when they run
        #[arg(long, default_value = "output")]
        outdir: String,
    },

    // Plumbing commands
    /// Template a test plan using provided values, outputting the resulting config to stdout
    Template {
        /// Relative path to the test-plan.yaml file that should be templated
        test_plan_path: String,
        /// Run a static check of the resulting test plan after templating
        #[arg(long, action)]
        check: bool,
    },
}
