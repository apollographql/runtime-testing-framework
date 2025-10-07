use crate::commands::summarise::OpSort;
use clap::{ArgAction, Parser, Subcommand};

/// A debbugging tool for looking into issues with supergraphs and routers
#[derive(Debug, Parser)]
#[clap(about, name = "rtfdoc", long_about = None)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Command,

    /// Flag to control logging verbosity. Default level is `warn`.
    /// `-v` sets logging level to `info`,`-vv` to `debug` and `-vvv` to `trace`.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    /// Output as JSON rather than markdown
    #[arg(long, global = true, action)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Summarise some top level information about a given supergraph in studio
    Summarise {
        /// The graph-ref to summarise
        #[arg(long)]
        graph_ref: String,

        /// Top operations over the last 30 days to summarise
        #[arg(short, long, default_value = "0")]
        n_operations: usize,

        /// Whether or not to include mutations when summarising operations
        #[arg(long, action)]
        skip_mutations: bool,

        /// How to order the queried operations (defaults to by most frequently run)
        #[arg(long, value_enum)]
        sort_by: Option<OpSort>,
    },

    /// Check the launch history of a given supergraph
    LaunchHistory {
        /// The graph-ref to look at launch history for
        #[arg(long)]
        graph_ref: String,

        /// The number of launches to check (successful & unsuccessful)
        #[arg(short, long, default_value = "100")]
        n: usize,
    },

    /// Summarise a historic launch of a given graph ref
    SchemaSummary {
        /// The graph-ref to look at launch history for
        #[arg(long)]
        graph_ref: String,

        /// The launch ID to summarise
        #[arg(long)]
        launch_id: String,
    },
}
