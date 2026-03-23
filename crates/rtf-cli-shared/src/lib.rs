use anyhow::Context;
use std::io::stderr;
use tracing::{Level, level_filters::LevelFilter, subscriber::set_global_default};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

/// Initialise our logger based on the given environment variable.
///
/// See the documentation on [EnvFilter] for details on how this works and what the supported
/// syntax is for setting a logging filter (it's a lot richer than just setting a level).
pub fn init_logging(env_var: &str, verbosity: u8) -> anyhow::Result<()> {
    // This is a bit of a song and dance to pull out what the max configured logging level is so we
    // can conditionally alter the output format we use when we are at INFO or above.
    // -> The thinking is that for the default case we want to restrict things to simple, compact
    //    log lines that don't overwhelm the user with too much information (mostly just calling
    //    out progress through the operation being performed). But, when things are dropped down to
    //    debug or trace we want to include more information such as the filename and timing
    //    information.
    let filter = EnvFilter::try_from_env(env_var).unwrap_or_else(|_| {
        // Map verbosity to tracing level string
        let level = match verbosity {
            0 => LevelFilter::WARN,
            1 => LevelFilter::INFO,
            2 => LevelFilter::DEBUG,
            _ => LevelFilter::TRACE,
        };

        // The hyper and h2 crates that we pull in have _very_ verbose logging that swamps
        // everything else and also is not typically helpful. We do this here where we construct
        // the filter explicitly in order to allow a user specified filter to enable these logs if
        // they are needed.
        EnvFilter::from_default_env()
            .add_directive(level.into())
            .add_directive("hyper=warn".parse().expect("valid directive"))
            .add_directive("h2=warn".parse().expect("valid directive"))
    });

    let max_level = filter
        .max_level_hint()
        .and_then(|l| l.into_level())
        .unwrap_or(Level::INFO);

    let builder = FmtSubscriber::builder()
        .with_env_filter(filter)
        .with_writer(stderr)
        .compact();

    // We can't just return a [tracing_subscriber::fmt::Subscriber] here (and then have a single
    // call to set_global_default) as it has a number of generics based on exactly how the builder
    // was run which means that each branch ends up returning a different type.
    if max_level <= Level::INFO {
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
