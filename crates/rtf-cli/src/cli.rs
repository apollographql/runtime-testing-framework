//! Parsing of our command line arguments using Clap's derive API
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use reqwest::{Method, Url};
use std::path::PathBuf;
use uuid::Uuid;

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

#[derive(Debug, Default, clap::Args)]
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

        #[command(flatten)]
        run_target: RunTarget,

        /// Execute a test plan file in GitHub instead of from a local path
        #[arg(long, default_value = "false")]
        github: bool,

        /// Optional git ref to pull files from when using --github
        #[arg(long = "ref", requires = "github")]
        git_ref: Option<String>,

        /// Output directory for providers when they run
        #[arg(long, default_value = "output")]
        outdir: String,

        /// Force removal of an existing output directory before running.
        #[arg(long, default_value = "false")]
        force: bool,
    },

    /// Open the RTF documentation in your browser
    #[command(alias = "m")]
    Docs {
        /// An optional search term to search for within the docs.
        search_term: Vec<String>,
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

    /// Resolve file providers for a config file without executing it
    Resolve {
        #[clap(subcommand)]
        subcommand: ResolveSubcommand,
    },

    /// Write a shell completion file to STDOUT for the given shell
    Completion {
        /// The shell to generate completions for (defaults to identifying from the environment)
        #[arg(long, short)]
        shell: Option<Shell>,
    },

    /// Output json schemas for environment configuration
    JsonSchemas { config: SchemasConfig },

    /// Interactions with the REP Orchestrator
    Rep {
        #[clap(subcommand)]
        subcommand: RepSubcommand,
    },

    /// Display CLI version and exit
    Version,
}

#[derive(Debug, clap::Args, Clone, Copy)]
#[group(required = false, multiple = false)]
pub struct RunTarget {
    /// Only run the environment setup
    #[arg(long)]
    pub environment_up: bool,

    /// Only run the environment teardown
    #[arg(long)]
    pub environment_down: bool,

    /// Only run the environment scenario
    #[arg(long)]
    pub scenario: bool,
}

impl RunTarget {
    /// Resolved flags for running setup, scenario & teardown
    pub fn as_flags(&self) -> (bool, bool, bool) {
        match (self.environment_up, self.scenario, self.environment_down) {
            // Clap ensures that we only ever have one of these flags set.
            // See https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html#argument-relations
            (true, _, _) => (true, false, false),
            (_, true, _) => (false, true, false),
            (_, _, true) => (false, false, true),
            // No flags being set means we run everything
            (false, false, false) => (true, true, true),
        }
    }
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

        /// Force removal of an existing output directory before running.
        #[arg(long, default_value = "false")]
        force: bool,
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

    /// Force removal of an existing output directory before running.
    #[arg(long, default_value = "false")]
    pub force: bool,

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

#[derive(Debug, Subcommand)]
pub enum ResolveSubcommand {
    /// Resolve file providers for a standalone scenario config
    Scenario {
        /// Relative path to the scenario.yaml file
        scenario_path: String,

        /// Output directory for resolved providers and scenario.env
        #[arg(long, default_value = "output")]
        outdir: String,

        /// Force removal of an existing output directory before running.
        #[arg(long, default_value = "false")]
        force: bool,
    },

    /// Resolve file providers for a standalone environment config
    Environment {
        /// Relative path to the environment.yaml file
        environment_path: String,

        /// Output directory for resolved providers and env files
        #[arg(long, default_value = "output")]
        outdir: String,

        /// Force removal of an existing output directory before running.
        #[arg(long, default_value = "false")]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepSubcommand {
    /// Prepare a test plan for execution by the REP service.
    /// Outputs a RepTestPlan JSON with inlined relative files and custom providers.
    Prepare {
        /// Relative path to the test plan file. When using --github this must be in the
        /// format ORG/REPO/PATH
        test_plan_path: String,

        /// Prepare a test plan file from GitHub instead of from a local path
        #[arg(long, default_value = "false")]
        github: bool,

        /// Optional git ref to pull files from when using --github
        #[arg(long = "ref", requires = "github")]
        git_ref: Option<String>,
    },

    /// Send an IAP-authenticated HTTP request to the REP orchestrator.
    ///
    /// The response body is written to stdout on success.
    Request {
        /// Path on the orchestrator to request (e.g. `/health`)
        path: String,

        /// HTTP method
        #[arg(short = 'X', long, default_value = "GET")]
        method: Method,

        /// Request body as a literal string
        #[arg(short, long)]
        body: Option<String>,

        /// Override the REP orchestrator base URL
        #[arg(long)]
        orchestrator_url: Option<Url>,
    },

    /// Trigger a test run using the REP orchestrator and poll for the result.
    ///
    /// The output of this command is aimed at being usable in CI runs and is non-interactive.
    CiRun {
        /// Relative path to the test plan file. When using --github this must be in the
        /// format ORG/REPO/PATH
        test_plan_path: String,

        /// Prepare a test plan file from GitHub instead of from a local path
        #[arg(long, default_value = "false")]
        github: bool,

        /// Optional git ref to pull files from when using --github
        #[arg(long = "ref", requires = "github")]
        git_ref: Option<String>,

        #[arg(long, default_value = "10")]
        poll_interval_seconds: u64,
    },

    /// Pull output for a single test execution
    ExecutionOutput {
        /// ID of the orchestrator test execution you wish to pull output for
        id: Uuid,

        /// Directory to place output in
        #[arg(long, default_value = "output")]
        outdir: String,

        /// Force removal of an existing output directory before running.
        #[arg(long, default_value = "false")]
        force: bool,
    },

    /// Pull output for all executions within a given test run
    RunOutput {
        /// ID of the orchestrator test run you wish to pull output for
        id: Uuid,

        /// Directory to place output in
        #[arg(long, default_value = "output")]
        outdir: String,

        /// Force removal of an existing output directory before running.
        #[arg(long, default_value = "false")]
        force: bool,
    },
}

#[derive(Debug, Clone, ValueEnum)]
pub enum SchemasConfig {
    TestPlan,
    Environment,
    Scenario,
}
