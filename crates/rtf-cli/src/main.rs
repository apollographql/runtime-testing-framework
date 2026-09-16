use clap::Parser;
use git_version::git_version;
use rtf_cli::{
    LOG_LEVEL_ENV_VAR,
    cli::{
        Args, Command, CustomProviderSubcommand, InlineSubcommand, RemoteSubcommand,
        ResolveSubcommand,
    },
    commands::{
        plumbing::{
            execute_remote_request, expand_test_plan_matrix, generate_json_schema,
            generate_shell_completions, inline_test_plan, resolve_environment, resolve_scenario,
            run_custom_provider, template_custom_provider, template_test_plan,
            test_custom_provider, write_remote_trigger_payload_to_stdout,
        },
        porcelain::{
            check_and_run_test_plan, ci_run, ci_run_known, execution_log, execution_status,
            open_docs, pull_execution_output, pull_run_output, remote_run, run_status,
        },
    },
};
use rtf_config::inlining::InlineMode;
use rustls::crypto::aws_lc_rs;
use std::process::exit;
use tracing::{error, warn};

#[tokio::main]
async fn main() {
    let Args {
        command,
        variables,
        verbose,
    } = Args::parse();

    if let Err(e) = rtf_cli_shared::init_logging(LOG_LEVEL_ENV_VAR, verbose) {
        error!("unable to initialise logging: {e}");
        exit(1);
    };

    if aws_lc_rs::default_provider().install_default().is_err() {
        panic!("unable to install default crypto provider");
    }

    // `rep` is a deprecated alias for `remote`. Normalise it here so the rest of `main` only
    // has to deal with one variant, and warn the user once so they can migrate.
    let command = match command {
        Command::Rep { subcommand } => {
            warn!(
                "`rtf rep` is deprecated and will be removed in a future release; use `rtf remote` instead"
            );

            Command::Remote { subcommand }
        }
        _ => command,
    };

    let res = match command {
        // porcelain commands
        Command::Run {
            test_plan_path,
            run_target,
            github,
            git_ref,
            outdir,
            force,
        } => {
            check_and_run_test_plan(
                &test_plan_path,
                github,
                git_ref,
                variables.into(),
                run_target,
                &outdir,
                force,
            )
            .await
        }

        Command::Docs { search_term } => open_docs(&search_term),

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
        } => template_test_plan(&test_plan_path, check, github, git_ref, variables.into()).await,

        Command::Inline { subcommand } => {
            let (args, mode) = match subcommand {
                InlineSubcommand::All { args } => (args, InlineMode::All),
                InlineSubcommand::RelativeFiles { args } => (args, InlineMode::RelativeFiles),
            };
            inline_test_plan(
                &args.test_plan_path,
                args.github,
                args.git_ref,
                variables.into(),
                &mode,
                &args.outdir,
                args.force,
            )
            .await
        }

        Command::CustomProvider {
            subcommand:
                CustomProviderSubcommand::Template {
                    definition_path,
                    check,
                },
        } => template_custom_provider(&definition_path, variables.into(), check).await,

        Command::CustomProvider {
            subcommand:
                CustomProviderSubcommand::Run {
                    definition_path,
                    outdir,
                    force,
                },
        } => run_custom_provider(&definition_path, variables.into(), &outdir, force).await,

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

        Command::Resolve {
            subcommand:
                ResolveSubcommand::Scenario {
                    scenario_path,
                    outdir,
                    force,
                },
        } => resolve_scenario(&scenario_path, variables.into(), &outdir, force).await,

        Command::Resolve {
            subcommand:
                ResolveSubcommand::Environment {
                    environment_path,
                    outdir,
                    force,
                },
        } => resolve_environment(&environment_path, variables.into(), &outdir, force).await,

        Command::Remote {
            subcommand:
                RemoteSubcommand::Request {
                    path,
                    method,
                    body,
                    plain_text,
                },
        } => execute_remote_request(&path, method, body.as_deref(), plain_text).await,

        Command::Remote {
            subcommand:
                RemoteSubcommand::Prepare {
                    test_plan_path,
                    github,
                    git_ref,
                },
        } => {
            write_remote_trigger_payload_to_stdout(
                &test_plan_path,
                github,
                git_ref,
                variables.into(),
            )
            .await
        }

        Command::Remote {
            subcommand:
                RemoteSubcommand::Run {
                    test_plan_path,
                    github,
                    git_ref,
                },
        } => remote_run(&test_plan_path, github, git_ref, variables.into()).await,

        Command::Remote {
            subcommand:
                RemoteSubcommand::CiRun {
                    test_plan_path,
                    github,
                    git_ref,
                    poll_interval_seconds,
                },
        } => {
            ci_run(
                &test_plan_path,
                github,
                git_ref,
                poll_interval_seconds,
                variables.into(),
            )
            .await
        }

        Command::Remote {
            subcommand:
                RemoteSubcommand::CiRunKnown {
                    test_plan_id,
                    git_ref,
                    poll_interval_seconds,
                },
        } => {
            ci_run_known(
                test_plan_id,
                git_ref,
                poll_interval_seconds,
                variables.into(),
            )
            .await
        }

        Command::Remote {
            subcommand: RemoteSubcommand::ExecutionLog { id },
        } => execution_log(id).await,

        Command::Remote {
            subcommand: RemoteSubcommand::ExecutionOutput { id, outdir, force },
        } => pull_execution_output(id, &outdir, force).await,

        Command::Remote {
            subcommand: RemoteSubcommand::ExecutionStatus { id },
        } => execution_status(id).await,

        Command::Remote {
            subcommand: RemoteSubcommand::RunOutput { id, outdir, force },
        } => pull_run_output(id, &outdir, force).await,

        Command::Remote {
            subcommand:
                RemoteSubcommand::RunStatus {
                    id,
                    with_executions,
                },
        } => run_status(id, with_executions).await,

        Command::Completion { shell } => generate_shell_completions(shell),

        Command::JsonSchemas { config } => generate_json_schema(config),

        Command::Version => {
            println!(
                "{}-{}",
                env!("CARGO_PKG_VERSION"),
                git_version!(fallback = "unknown")
            );
            exit(0);
        }

        // `rep` is normalised to `remote` above, before this match runs.
        Command::Rep { .. } => unreachable!(),
    };

    if let Err(e) = res {
        error!("{e}");
        exit(1);
    }
}
