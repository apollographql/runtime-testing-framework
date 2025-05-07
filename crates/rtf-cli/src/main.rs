use anyhow::Context;
use clap::Parser;
use rtf_cli::{
    cli::{Args, Command},
    commands::{fetch_supergraph, top_operations},
};
use std::io::stdout;
use tracing::{Level, level_filters::LevelFilter, subscriber::set_global_default};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging()?;
    let args = Args::parse();

    match args.command {
        Command::FetchSupergraph {
            graph_id,
            variant,
            out_dir,
            staging,
        } => fetch_supergraph(graph_id, variant, out_dir, staging).await?,

        Command::TopOperations {
            graph_id,
            variant,
            n_operations,
            skip_mutations,
            out_dir,
            staging,
        } => {
            top_operations(
                graph_id,
                variant,
                n_operations,
                skip_mutations,
                out_dir,
                staging,
            )
            .await?
        }
    }

    Ok(())
}

/// Initialise our logger based on the RUST_LOG environment variable.
///
/// See the documentation on [EnvFilter] for details on how this works and what the supported
/// syntax is for setting a logging filter (it's a lot richer than just setting a level).
fn init_logging() -> anyhow::Result<()> {
    // This is a bit of a song and dance to pull out what the max configured logging level is so we
    // can conditionally alter the output format we use when we are at INFO or above.
    // -> The thinking is that for the default case we want to restrict things to simple, compact
    //    log lines that don't overwhelm the user with too much information (mostly just calling
    //    out progress through the operation being performed). But, when things are dropped down to
    //    debug or trace we want to include more information such as the filename and timing
    //    information.
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    let max_level = filter
        .max_level_hint()
        .and_then(|l| l.into_level())
        .unwrap_or(Level::INFO);

    let builder = FmtSubscriber::builder()
        .with_env_filter(filter)
        .with_writer(stdout)
        .compact();

    // We can't just return a [tracing_subscriber::fmt::Subscriber] here (and then have a single
    // call to set_global_default) as it has a number of generics based on exactly how the builder
    // was run which means that each branch ends up returning a different type.
    if max_level >= Level::INFO {
        // Opinionated log formatting: minimising the output as much as possible by default
        let subscriber = builder
            .with_target(false)
            .with_file(false)
            .without_time()
            .finish();
        set_global_default(subscriber).context("unable to set a global tracing subscriber")?;
    } else {
        let subscriber = builder.finish();
        set_global_default(subscriber).context("unable to set a global tracing subscriber")?;
    };

    Ok(())
}
