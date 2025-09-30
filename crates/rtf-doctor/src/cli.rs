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

        /// Order operations by number of fields being queried
        #[arg(long, action)]
        by_fields: bool,
    },
}
