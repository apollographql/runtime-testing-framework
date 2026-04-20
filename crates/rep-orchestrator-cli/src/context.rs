use crate::{
    error::{CliError, CliResult},
    kubernetes,
    orchestrator::{self, Client},
};
use anyhow::Context;
use rep_orchestrator_shared::{
    EXECUTION_ID_ENV_VAR, EXECUTION_TOKEN_ENV_VAR, ORCHESTRATOR_URL_ENV_VAR, status::Status,
};
use std::{env, path::Path, process::Command, str::FromStr};
use tracing::info;
use uuid::Uuid;

pub trait CliContext {
    type OrchestratorClient: orchestrator::Client;
    type KubeClient: kubernetes::Client;

    fn orchestrator_client(&self) -> &Self::OrchestratorClient;

    fn kube_client(&self) -> &Self::KubeClient;

    /// Run a shell [Command] to completion.
    ///
    /// If it exits with a successful exit code, [Self::OrchestratorClient] will post an update moving the associated
    /// test execution to `next_execution_status`.
    ///
    /// If it fails, [Self::OrchestratorClient] will be used to update the status of the associated test execution to either
    /// a [Status::Unrunnable] or [Status::Failed] state depending on whether the failed command was associated with invoking
    /// user scripts or setup scripts.
    async fn run_shell(&self, cmd: &mut Command, next_execution_status: Status) -> CliResult<()> {
        info!("Running {cmd:?}");

        let is_orchestration = matches!(
            next_execution_status,
            Status::Initialising | Status::Resolving | Status::Provisioning
        );

        let exit_status = cmd
            .status()
            .with_context(|| format!("I/O error when attempting to invoke {cmd:?}"))
            .map_err(CliError::unrunnable)?;

        match exit_status.success() {
            false if is_orchestration => Err(CliError::unrunnable_subprocess(
                exit_status,
                format!("Setup command failed: {cmd:?}"),
            )),
            false => Err(CliError::failed(
                exit_status,
                format!("Test plan invocation failed: {cmd:?}"),
            )),
            true => {
                self.orchestrator_client()
                    .update_status(
                        next_execution_status,
                        Some(exit_status),
                        Some(format!("Completed command: {cmd:?}")),
                    )
                    .await
                    .context("Failed to update execution status")
                    .map_err(CliError::unrunnable)?;

                Ok(())
            }
        }
    }
}

pub struct EnvironmentContext {
    orchestrator_client: orchestrator::HttpClient,
    kube_client: kubernetes::HttpClient,
}

impl EnvironmentContext {
    pub async fn from_environment(kubeconfig: Option<&Path>) -> anyhow::Result<Self> {
        let orchestrator_url = env::var(ORCHESTRATOR_URL_ENV_VAR)
            .context(format!("{ORCHESTRATOR_URL_ENV_VAR} must be set"))?;

        let execution_id = env::var(EXECUTION_ID_ENV_VAR)
            .context(format!("{EXECUTION_ID_ENV_VAR} must be set"))
            .and_then(|id_var| {
                Uuid::from_str(&id_var)
                    .context(format!("{EXECUTION_ID_ENV_VAR} must be a valid UUID"))
            })?;

        let execution_token = env::var(EXECUTION_TOKEN_ENV_VAR)
            .context(format!("{EXECUTION_TOKEN_ENV_VAR} must be set"))
            .and_then(|token_var| {
                Uuid::from_str(&token_var)
                    .context(format!("{EXECUTION_TOKEN_ENV_VAR} must be a valid UUID"))
            })?;

        let orchestrator_client =
            orchestrator::HttpClient::new(orchestrator_url, execution_id, execution_token);
        let kube_client = kubernetes::HttpClient::from_kubeconfig(kubeconfig).await?;

        Ok(Self {
            orchestrator_client,
            kube_client,
        })
    }
}

impl CliContext for EnvironmentContext {
    type OrchestratorClient = orchestrator::HttpClient;
    type KubeClient = kubernetes::HttpClient;

    fn orchestrator_client(&self) -> &Self::OrchestratorClient {
        &self.orchestrator_client
    }

    fn kube_client(&self) -> &Self::KubeClient {
        &self.kube_client
    }
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use kubernetes::mocks::MockClient as MockKubeClient;
    use orchestrator::mocks::MockClient as MockOrchestrator;

    #[derive(Default)]
    pub struct MockContext {
        pub orchestrator_client: MockOrchestrator,
        pub kube_client: MockKubeClient,
    }

    impl CliContext for MockContext {
        type OrchestratorClient = MockOrchestrator;
        type KubeClient = MockKubeClient;

        fn orchestrator_client(&self) -> &Self::OrchestratorClient {
            &self.orchestrator_client
        }

        fn kube_client(&self) -> &Self::KubeClient {
            &self.kube_client
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::mocks::MockContext, orchestrator::mocks::MockClient as MockOrchestrator};
    use rep_orchestrator_shared::status::Status;
    use simple_test_case::test_case;

    #[tokio::test]
    async fn run_shell_updates_status_on_success() {
        let ctx = MockContext::default();
        let result = ctx
            .run_shell(&mut Command::new("true"), Status::Provisioning)
            .await;

        assert!(result.is_ok());
        ctx.orchestrator_client().read_updates(|updates| {
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].status, Status::Provisioning);
            assert!(updates[0].exit_status.unwrap().success());
        });
    }

    #[tokio::test]
    async fn run_shell_includes_command_in_status_message() {
        let ctx = MockContext::default();
        ctx.run_shell(&mut Command::new("true"), Status::Running)
            .await
            .unwrap();

        ctx.orchestrator_client().read_updates(|updates| {
            let message = updates[0].message.as_deref().unwrap();
            assert_eq!(
                message, "Completed command: \"true\"",
                "expected command in message: {message}"
            );
        });
    }

    #[test_case(Status::Initialising, Status::Unrunnable; "initialising maps to unrunnable")]
    #[test_case(Status::Resolving, Status::Unrunnable; "resolving maps to unrunnable")]
    #[test_case(Status::Provisioning, Status::Unrunnable; "provisioning maps to unrunnable")]
    #[test_case(Status::Running, Status::Failed; "running maps to failed")]
    #[tokio::test]
    async fn run_shell_failed_command_maps_to_expected_status(
        input_status: Status,
        expected_error_status: Status,
    ) {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), input_status)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), expected_error_status);
        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()));
    }

    #[test_case(Status::Running, "Test plan invocation failed"; "failed includes invocation context")]
    #[test_case(Status::Provisioning, "Setup command failed"; "unrunnable includes setup context")]
    #[tokio::test]
    async fn run_shell_error_message_includes_context(status: Status, expected_context: &str) {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), status)
            .await
            .unwrap_err();

        let msg = err.source_to_string();
        assert!(msg.contains("false"), "expected command in error: {msg}");
        assert!(
            msg.contains(expected_context),
            "expected '{expected_context}' in error: {msg}"
        );
    }

    #[tokio::test]
    async fn run_shell_returns_unrunnable_on_io_error() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(
                &mut Command::new("this-binary-definitely-does-not-exist"),
                Status::Running,
            )
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()));
    }

    #[tokio::test]
    async fn run_shell_returns_unrunnable_when_status_update_fails() {
        let ctx = MockContext {
            orchestrator_client: MockOrchestrator::failing(),
            ..Default::default()
        };

        let err = ctx
            .run_shell(&mut Command::new("true"), Status::Provisioning)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }
}
