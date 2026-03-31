mod cli;
mod commands;
mod context;
mod error;
mod orchestrator;

use crate::{
    context::{CliContext, EnvironmentContext},
    orchestrator::Client,
};
use anyhow::{anyhow, bail};
use clap::Parser;
use cli::{Args, Command};
use tracing::error;

const LOG_LEVEL_ENV_VAR: &str = "APOLLO_REP_ORCHESTRATOR_LOG";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args { command, verbose } = Args::parse();

    // Unlike rtf-cli, we start at INFO as our default log level
    if let Err(e) = rtf_cli_shared::init_logging(LOG_LEVEL_ENV_VAR, verbose + 1) {
        bail!("unable to initialise logging: {e}");
    };

    let ctx = match EnvironmentContext::from_environment() {
        Err(e) => {
            bail!("unable to initialize REP Orchestrator CLI: {e}");
        }
        Ok(context) => context,
    };

    run_command(command, &ctx).await
}

async fn run_command(command: Command, ctx: &impl CliContext) -> anyhow::Result<()> {
    let res = match command {
        Command::CreateNamespace {
            namespace,
            kubeconfig: kubeconfig_path,
        } => commands::create_namespace(&namespace, &kubeconfig_path).await,

        Command::CreatePullSecret {
            namespace,
            kubeconfig: kubeconfig_path,
            docker_config: docker_config_path,
        } => commands::create_pull_secret(&namespace, &kubeconfig_path, &docker_config_path).await,

        Command::DeployEnvironment {
            namespace,
            kubeconfig: kubeconfig_path,
            environment: environment_path,
            timeout,
        } => {
            commands::deploy_environment(
                &namespace,
                &kubeconfig_path,
                &environment_path,
                timeout,
                ctx,
            )
            .await
        }

        Command::Cleanup {
            configmap,
            namespace,
        } => commands::cleanup(&configmap, &namespace).await,
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
    use crate::orchestrator::mocks::MockClient as MockOrchestrator;
    use rep_orchestrator_shared::status::Status;
    use std::path::PathBuf;

    fn failing_create_namespace() -> Command {
        Command::CreateNamespace {
            namespace: "test-ns".to_owned(),
            kubeconfig: PathBuf::from("/dev/null"),
        }
    }

    #[tokio::test]
    async fn command_error_triggers_status_update() {
        let ctx = MockContext::default();
        let result = run_command(failing_create_namespace(), &ctx).await;

        assert!(result.is_err());
        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].status, Status::Unrunnable);
        });
    }

    #[tokio::test]
    async fn command_error_message_propagated_to_caller() {
        let ctx = MockContext::default();
        let err = run_command(failing_create_namespace(), &ctx)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("failed to build kube config"),
            "expected kubeconfig error: {err}"
        );
    }

    #[tokio::test]
    async fn status_update_failure_does_not_mask_command_error() {
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::failing(),
        };
        let err = run_command(failing_create_namespace(), &ctx)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("failed to build kube config"),
            "expected original error despite status update failure: {err}"
        );
    }
}
