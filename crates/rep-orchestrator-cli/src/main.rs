use crate::{
    context::{CliContext, EnvironmentContext},
    orchestrator::Client,
};
use anyhow::{anyhow, bail};
use clap::Parser;
use cli::{Args, Command};
use tracing::error;

mod cli;
mod commands;
mod context;
mod error;
mod kubernetes;
mod orchestrator;
mod status;

const LOG_LEVEL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_LOG";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args { command, verbose } = Args::parse();

    // Unlike rtf-cli, we start at INFO as our default log level
    if let Err(e) = rtf_cli_shared::init_logging(LOG_LEVEL_ENV_VAR, verbose + 1) {
        bail!("unable to initialise logging: {e}");
    };

    let ctx = match EnvironmentContext::from_environment(command.kubeconfig()).await {
        Err(e) => {
            bail!("unable to initialize REP Orchestrator CLI: {e}");
        }
        Ok(context) => context,
    };

    run_command(command, &ctx).await
}

async fn run_command(command: Command, ctx: &impl CliContext) -> anyhow::Result<()> {
    let res = match command {
        Command::CreateNamespace { namespace, .. } => {
            commands::create_namespace(&namespace, ctx).await
        }

        Command::CreateServiceAccount { namespace, .. } => {
            commands::create_service_account(&namespace, ctx).await
        }

        Command::DeployEnvironment {
            namespace,
            kubeconfig: kubeconfig_path,
            timeout,
            provider_dir,
            toolbox_pull_policy,
        } => {
            commands::deploy_environment(
                &namespace,
                &kubeconfig_path,
                &provider_dir,
                &toolbox_pull_policy,
                timeout,
                ctx,
            )
            .await
        }

        Command::ResolveEnvironment { outdir } => commands::resolve_environment(&outdir, ctx).await,

        Command::PrepareScenario {
            shared_dir,
            command,
        } => commands::prepare_scenario(&shared_dir, &command, ctx).await,

        Command::CollectOutput { shared_dir } => commands::collect_output(&shared_dir, ctx).await,
    };

    match res {
        Ok(()) => Ok(()),
        Err(e) => {
            let console_err = anyhow!(e.source_to_string());
            if let Err(status_failure) = ctx.orchestrator_client().update_error_status(e).await {
                error!("Failed to update status for error: {status_failure}");
            }
            Err(console_err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::mocks::MockContext;
    use crate::kubernetes::mocks::MockClient as MockKubeClient;
    use crate::orchestrator::mocks::MockClient as MockOrchestrator;
    use rep_orchestrator_shared::status::Status;
    use simple_test_case::test_case;
    use std::path::PathBuf;

    fn create_namespace_command() -> Command {
        Command::CreateNamespace {
            namespace: "test-ns".to_owned(),
            kubeconfig: PathBuf::from("/dev/null"),
        }
    }

    enum FailingClient {
        Orchestrator,
        Kube,
    }

    fn build_context(failing_client: FailingClient) -> MockContext {
        match failing_client {
            FailingClient::Orchestrator => MockContext {
                orchestrator_client: MockOrchestrator::failing(),
                ..Default::default()
            },
            FailingClient::Kube => MockContext {
                kube_client: MockKubeClient::failing(),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn command_error_triggers_status_update() {
        let ctx = build_context(FailingClient::Kube);
        let result = run_command(create_namespace_command(), &ctx).await;

        assert!(result.is_err());
        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[0].status, Status::Provisioning);
            assert_eq!(updates[1].status, Status::Unrunnable);
        });
    }

    #[test_case(FailingClient::Kube, "Failed to create kube namespace"; "command error propagated to caller")]
    #[test_case(FailingClient::Orchestrator, "mock update failure"; "status update failure does not mask command error")]
    #[tokio::test]
    async fn error_message_contains_original_cause(failing_client: FailingClient, expected: &str) {
        let ctx = build_context(failing_client);
        let err = run_command(create_namespace_command(), &ctx)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains(expected),
            "expected '{expected}' in error: {err}"
        );
    }
}
