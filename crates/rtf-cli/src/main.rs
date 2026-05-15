use clap::Parser;
use git_version::git_version;
use rtf_cli::{
    LOG_LEVEL_ENV_VAR,
    cli::{
        Args, Command, CustomProviderSubcommand, InlineSubcommand, RepSubcommand, ResolveSubcommand,
    },
    commands::{
        plumbing::{
            execute_rep_request, expand_test_plan_matrix, generate_json_schema,
            generate_shell_completions, inline_test_plan, resolve_environment, resolve_scenario,
            run_custom_provider, template_custom_provider, template_test_plan,
            test_custom_provider, write_rep_trigger_payload_to_stdout,
        },
        porcelain::{check_and_run_test_plan, open_docs},
    },
};
use rtf_config::inlining::InlineMode;
use rustls::crypto::aws_lc_rs;
use std::process::exit;
use tracing::error;

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

        Command::Rep {
            subcommand:
                RepSubcommand::Request {
                    path,
                    method,
                    body,
                    orchestrator_url,
                },
        } => execute_rep_request(&path, method, body.as_deref(), orchestrator_url).await,

        Command::Rep {
            subcommand:
                RepSubcommand::Prepare {
                    test_plan_path,
                    github,
                    git_ref,
                },
        } => {
            write_rep_trigger_payload_to_stdout(&test_plan_path, github, git_ref, variables.into())
                .await
        }

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
    };

    if let Err(e) = res {
        error!("{e}");
        exit(1);
    }
}
