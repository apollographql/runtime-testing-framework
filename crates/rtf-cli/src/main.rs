use anyhow::Context;
use clap::Parser;
use rtf_cli::{
    LOG_LEVEL_ENV_VAR,
    cli::{Args, Command, CustomProviderSubcommand},
    commands::{
        plumbing::{
            expand_test_plan_matrix, run_custom_provider, template_custom_provider,
            template_test_plan_github, template_test_plan_local, test_custom_provider,
        },
        porcelain::check_and_run_test_plan,
    },
};
use std::{io::stderr, process::exit};
use tracing::{Level, error, level_filters::LevelFilter, subscriber::set_global_default};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tokio::main]
async fn main() {
    let Args {
        command,
        variables,
        verbose,
    } = Args::parse();

    if let Err(e) = init_logging(verbose) {
        error!("unable to initialise logging: {e}");
        exit(1);
    };

    let res = match command {
        // porcelain commands
        Command::Run {
            test_plan_path,
            github,
            git_ref,
            outdir,
        } => check_and_run_test_plan(&test_plan_path, github, git_ref, variables, &outdir).await,

        // plumbing commands
        Command::ExpandMatrix {
            test_plan_path,
            compact,
        } => expand_test_plan_matrix(&test_plan_path, compact).await,

        Command::Template {
            test_plan_path,
            check,
            github,
            git_ref,
        } => match (test_plan_path, github, git_ref) {
            (Some(path), None, None) => template_test_plan_local(&path, variables, check).await,
            (Some(_), None, Some(_)) => {
                error!("--ref is not supported for local file paths");
                exit(1)
            }
            (None, Some(org_repo_path), git_ref) => {
                template_test_plan_github(org_repo_path, git_ref, variables, check).await
            }
            (None, None, _) => {
                error!("no test plan provided");
                exit(1)
            }
            (Some(_), Some(_), _) => unreachable!(),
        },

        Command::CustomProvider {
            subcommand:
                CustomProviderSubcommand::Template {
                    definition_path,
                    check,
                },
        } => template_custom_provider(&definition_path, variables, check).await,

        Command::CustomProvider {
            subcommand:
                CustomProviderSubcommand::Run {
                    definition_path,
                    outdir,
                },
        } => run_custom_provider(&definition_path, variables, &outdir).await,

        Command::CustomProvider {
            subcommand:
                CustomProviderSubcommand::Test {
                    definition_path,
                    test_cases_dir,
                    error_on_empty,
                    no_capture,
                },
        } => {
            test_custom_provider(&definition_path, test_cases_dir, error_on_empty, no_capture).await
        }
    };

    if let Err(e) = res {
        error!("{e}");
        exit(1);
    }
}

/// Initialise our logger based on the [LOG_LEVEL_ENV_VAR] environment variable.
///
/// See the documentation on [EnvFilter] for details on how this works and what the supported
/// syntax is for setting a logging filter (it's a lot richer than just setting a level).
fn init_logging(verbosity: u8) -> anyhow::Result<()> {
    // This is a bit of a song and dance to pull out what the max configured logging level is so we
    // can conditionally alter the output format we use when we are at INFO or above.
    // -> The thinking is that for the default case we want to restrict things to simple, compact
    //    log lines that don't overwhelm the user with too much information (mostly just calling
    //    out progress through the operation being performed). But, when things are dropped down to
    //    debug or trace we want to include more information such as the filename and timing
    //    information.
    let filter = EnvFilter::try_from_env(LOG_LEVEL_ENV_VAR).unwrap_or_else(|_| {
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
