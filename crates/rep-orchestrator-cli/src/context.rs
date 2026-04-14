use crate::{
    error::{CliError, CliResult},
    orchestrator::{self, Client},
};
use anyhow::Context;
use rep_orchestrator_shared::{EXECUTION_ID_ENV_VAR, ORCHESTRATOR_URL_ENV_VAR, status::Status};
use std::{env, process::Command, str::FromStr};
use tracing::info;
use uuid::Uuid;

pub trait CliContext {
    type OrchestratorClient: orchestrator::Client;

    fn orchestrator_client(&self) -> &Self::OrchestratorClient;

    /// Run a shell [Command] to completion.
    ///
    /// If it exits with a successful exit code, [Self::OrchestratorClient] will post an update moving the associated
    /// test execution to `next_execution_status`.
    ///
    /// If it fails, [Self::OrchestratorClient] will be used to update the status of the associated test execution to either
    /// a [Status::Unrunnable] or [Status::Failed] state depending on whether the failed command was associated with invoking
    /// user scripts or setup scripts.
    async fn run_shell(&self, cmd: &mut Command, next_execution_status: Status) -> CliResult<()> {
        let cmd_context = format!("running: {cmd:?}");
        info!("{cmd_context}");

        let is_orchestration = matches!(
            next_execution_status,
            Status::Initialising | Status::Resolving | Status::Provisioning
        );

        let exit_status = cmd
            .status()
            .with_context(|| format!("I/O error when attempting to invoke {cmd_context}"))
            .map_err(CliError::unrunnable)?;

        match exit_status.success() {
            false if is_orchestration => Err(CliError::unrunnable_subprocess(
                exit_status,
                format!("Setup command failed: {cmd_context}"),
            )),
            false => Err(CliError::failed(
                exit_status,
                format!("Test plan invocation failed: {cmd_context}"),
            )),
            true => {
                self.orchestrator_client()
                    .update_status(next_execution_status, Some(exit_status), Some(cmd_context))
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
}

impl EnvironmentContext {
    pub fn from_environment() -> anyhow::Result<Self> {
        let orchestrator_url = env::var(ORCHESTRATOR_URL_ENV_VAR)
            .context(format!("{ORCHESTRATOR_URL_ENV_VAR} must be set"))?;

        let execution_id = env::var(EXECUTION_ID_ENV_VAR)
            .context(format!("{EXECUTION_ID_ENV_VAR} must be set"))
            .and_then(|id_var| {
                Uuid::from_str(&id_var)
                    .context(format!("{EXECUTION_ID_ENV_VAR} must be a valid UUID"))
            })?;

        let orchestrator_client = orchestrator::HttpClient::new(orchestrator_url, execution_id);

        Ok(Self {
            orchestrator_client,
        })
    }
}

impl CliContext for EnvironmentContext {
    type OrchestratorClient = orchestrator::HttpClient;

    fn orchestrator_client(&self) -> &Self::OrchestratorClient {
        &self.orchestrator_client
    }
}

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use orchestrator::mocks::MockClient as MockOrchestrator;

    #[derive(Default)]
    pub struct MockContext {
        pub orchestrator_client: MockOrchestrator,
    }

    impl CliContext for MockContext {
        type OrchestratorClient = MockOrchestrator;

        fn orchestrator_client(&self) -> &Self::OrchestratorClient {
            &self.orchestrator_client
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::mocks::MockContext, orchestrator::mocks::MockClient as MockOrchestrator};
    use rep_orchestrator_shared::status::Status;

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
                message, "running: \"true\"",
                "expected command in message: {message}"
            );
        });
    }

    #[tokio::test]
    async fn run_shell_returns_unrunnable_for_failed_initialising_command() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Initialising)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()));
    }

    #[tokio::test]
    async fn run_shell_returns_unrunnable_for_failed_resolving_command() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Resolving)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }

    #[tokio::test]
    async fn run_shell_returns_unrunnable_for_failed_provisioning_command() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Provisioning)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }

    #[tokio::test]
    async fn run_shell_returns_failed_for_non_orchestration_command() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Running)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Failed);
        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()))
    }

    #[tokio::test]
    async fn run_shell_failed_error_message_includes_command() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Running)
            .await
            .unwrap_err();

        let msg = err.source_to_string();
        assert!(msg.contains("false"), "expected command in error: {msg}");
        assert!(
            msg.contains("Test plan invocation failed"),
            "expected failure context: {msg}"
        );
    }

    #[tokio::test]
    async fn run_shell_unrunnable_error_message_includes_setup_context() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Provisioning)
            .await
            .unwrap_err();

        let msg = err.source_to_string();
        assert!(
            msg.contains("Setup command failed"),
            "expected setup context: {msg}"
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
        };

        let err = ctx
            .run_shell(&mut Command::new("true"), Status::Provisioning)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }
}
