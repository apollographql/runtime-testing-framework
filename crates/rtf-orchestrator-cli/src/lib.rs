use crate::{cli::Command, context::CliContext, orchestrator::Client};
use anyhow::anyhow;
use rtf_orchestrator_shared::{OtelConfig, status::Status};
use tracing::error;

mod cli;
mod commands;
mod context;
mod error;
mod kubernetes;
mod orchestrator;
mod status;

pub use cli::Args;
pub use context::EnvironmentContext;
pub use error::{Error, Result};

pub async fn run_command(command: Command, ctx: &impl CliContext) -> anyhow::Result<()> {
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
            otel_collector_grpc,
            otel_collector_http,
        } => {
            commands::deploy_environment(
                &namespace,
                &kubeconfig_path,
                &provider_dir,
                &toolbox_pull_policy,
                &OtelConfig {
                    grpc: otel_collector_grpc.clone(),
                    http: otel_collector_http.clone(),
                },
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

        // Collect output is unique in the fact that it can set a Failed status where all other CLI
        // fatal errors result in Unrunnable.
        Command::CollectOutput {
            shared_dir,
            prometheus_endpoint,
        } => {
            let e = match commands::collect_output(&shared_dir, &prometheus_endpoint, ctx).await {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };

            let (status, exit_status) = e.status_and_exit_status();
            let msg = e.to_string();

            ctx.orchestrator_client()
                .update_status(status, exit_status, Some(msg.clone()))
                .await?;

            return Err(anyhow!(msg));
        }
    };

    match res {
        Ok(()) => Ok(()),
        Err(e) => {
            let console_err = anyhow!(e.to_string());
            if let Err(status_failure) = ctx
                .orchestrator_client()
                .update_status(Status::Unrunnable, None, Some(e.to_string()))
                .await
            {
                error!("failed to update status for error: {status_failure}");
            }

            Err(console_err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::mocks::MockContext, kubernetes::mocks::MockClient as MockKubeClient,
        orchestrator::mocks::MockClient as MockOrchestrator,
    };
    use rtf_orchestrator_shared::status::Status;
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
            assert_eq!(updates[0], Status::Provisioning);
            assert_eq!(updates[1], Status::Unrunnable);
        });
    }

    #[test_case(FailingClient::Kube, "failed to create namespace"; "command error propagated to caller")]
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
