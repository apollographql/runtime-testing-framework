//! Parsing of our command line arguments using Clap's derive API
use clap::{ArgAction, Parser, Subcommand};
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
    pub variables: Variables,

    /// Flag to control logging verbosity. Default level is `warn`.
    /// `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, clap::Args)]
pub struct Variables {
    /// A single additional templating variable in the form "key=value"
    #[arg(long, global = true, alias = "value")]
    // This alias is for backwards compatibility with the original flag
    // It is hidden from the user documentation
    pub var: Vec<String>,

    /// Path to a JSON file containing additional template variables
    #[arg(long, global = true, alias = "values")]
    // This alias is for backwards compatibility with the original flag
    // It is hidden from the user documentation
    pub vars: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // Porcelain commands
    /// Check and run a test plan
    Run {
        /// Relative path to the test plan file that should be executed. When using --github this must be in the format ORG/REPO/PATH
        test_plan_path: String,

        /// Execute a test plan file in GitHub instead of from a local path
        #[arg(long, default_value = "false")]
        github: bool,

        /// Optional git ref to pull files from when using --github
        #[arg(long = "ref", requires = "github")]
        git_ref: Option<String>,

        /// Output directory for providers when they run
        #[arg(long, default_value = "output")]
        outdir: String,
    },

    // Plumbing commands
    /// Expand a test plan matrix into JSON
    ExpandMatrix {
        /// Relative path to the test-plan.yaml file that should have its matrix expanded
        test_plan_path: String,

        /// Return the expanded matrix JSON in compact form
        #[arg(long, short, action)]
        compact: bool,
    },

    /// Template a test plan using provided variables, outputting the resulting config to stdout
    Template {
        /// Relative path to the test plan file that should be templated. When using --github this must be in the format ORG/REPO/PATH
        test_plan_path: String,

        /// Run a static check of the resulting test plan after templating
        #[arg(long, action)]
        check: bool,

        /// Template a test plan file from GitHub instead of from a local path
        #[arg(long, default_value = "false")]
        github: bool,

        /// Optional git ref to pull files from when using --github
        #[arg(long = "ref", requires = "github")]
        git_ref: Option<String>,
    },

    /// Work directly with custom file provider definitions
    CustomProvider {
        #[clap(subcommand)]
        subcommand: CustomProviderSubcommand,
    },

    /// Inline file providers in a test plan.
    /// Outputs the resulting test plan to the given directory.
    Inline {
        #[clap(subcommand)]
        subcommand: InlineSubcommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum CustomProviderSubcommand {
    /// Template a custom provider definition, outputting the resulting config to stdout
    Template {
        /// Relative path to the custom provider definition file
        definition_path: String,

        /// Run a static check of the resulting test plan after templating
        #[arg(long, action)]
        check: bool,
    },

    /// Execute a custom provider definition
    Run {
        /// Relative path to the custom provider definition file
        definition_path: String,

        /// Output directory for provider execution
        #[arg(long, default_value = "output")]
        outdir: String,
    },

    /// !!EXPERIMENTAL!! Run tests for the given provider
    #[command(hide = true)]
    Test {
        /// Relative path to the custom provider definition file
        definition_path: String,

        /// The directory that contains the test cases
        #[arg(long)]
        test_cases_dir: Option<String>,

        /// Whether or not having zero test cases is considered an error
        #[arg(long, action)]
        error_on_empty: bool,

        /// Show captured stdout/stderr for failed tests
        #[arg(long, action)]
        no_capture: bool,
    },
}

#[derive(Debug, clap::Args)]
pub struct InlineArgs {
    /// Relative path to the test-plan.yaml file that should be inlined. When using --github this must be in the format ORG/REPO/PATH
    pub test_plan_path: String,

    /// Output directory for inlined test plan
    #[arg(long, default_value = "output")]
    pub outdir: String,

    /// Inline a test plan file from GitHub instead of from a local path
    #[arg(long, default_value = "false")]
    pub github: bool,

    /// Optional git ref to pull files from when using --github
    #[arg(long = "ref", requires = "github")]
    pub git_ref: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum InlineSubcommand {
    /// Inline all file providers
    All {
        #[command(flatten)]
        args: InlineArgs,
    },
    /// Inline only relative file providers
    RelativeFiles {
        #[command(flatten)]
        args: InlineArgs,
    },
}
