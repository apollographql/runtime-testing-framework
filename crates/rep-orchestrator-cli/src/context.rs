use crate::{
    error::{CliError, CliResult},
    kubernetes,
    orchestrator::{self, Client},
};
use anyhow::Context;
use rep_orchestrator_shared::{
    EXECUTION_ID_ENV_VAR, EXECUTION_TOKEN_ENV_VAR, ORCHESTRATOR_URL_ENV_VAR, status::Status,
};
use std::{
    env,
    fs::{self, Permissions, set_permissions},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};
use tracing::info;
use uuid::Uuid;

pub trait CliContext {
    type OrchestratorClient: orchestrator::Client;
    type KubeClient: kubernetes::Client;

    fn orchestrator_client(&self) -> &Self::OrchestratorClient;

    fn kube_client(&self) -> &Self::KubeClient;

    /// Run an orchestration shell [Command] to completion.
    ///
    /// On success, posts an update moving the execution to `next_execution_status`. On failure,
    /// returns a [Status::Unrunnable] error — every caller is an orchestration subroutine, so
    /// there is no distinction between "setup failed" and "user scenario failed" here.
    async fn run_shell(&self, cmd: &mut Command, next_execution_status: Status) -> CliResult<()> {
        info!("Running {cmd:?}");

        let exit_status = cmd
            .status()
            .with_context(|| format!("I/O error when attempting to invoke {cmd:?}"))
            .map_err(CliError::unrunnable)?;

        if !exit_status.success() {
            return Err(CliError::unrunnable_subprocess(
                exit_status,
                format!("Command failed: {cmd:?}"),
            ));
        }

        self.orchestrator_client()
            .update_status(
                next_execution_status,
                None,
                Some(format!("Completed command: {cmd:?}")),
            )
            .await
            .map_err(CliError::unrunnable)?;

        Ok(())
    }

    /// Read the file at `path` as UTF-8, mapping the IO error to [`Status::Unrunnable`] with the
    /// path embedded in the message.
    fn read_file_to_string(&self, path: &Path) -> CliResult<String> {
        fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))
            .map_err(CliError::unrunnable)
    }

    /// Read the file at `path` as bytes, mapping the IO error to [`Status::Unrunnable`] with the
    /// path embedded in the message.
    fn read_file(&self, path: &Path) -> CliResult<Vec<u8>> {
        fs::read(path)
            .with_context(|| format!("Failed to read {}", path.display()))
            .map_err(CliError::unrunnable)
    }

    /// Write `content` to `path`, mapping the IO error to [`Status::Unrunnable`] with the path
    /// embedded in the message.
    fn write_file(&self, path: &Path, content: &[u8]) -> CliResult<()> {
        fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))
            .map_err(CliError::unrunnable)
    }

    /// chmod `path` to `mode`, mapping the IO error to [`Status::Unrunnable`] with the path
    /// embedded in the message.
    fn set_permissions_mode(&self, path: &Path, mode: u32) -> CliResult<()> {
        set_permissions(path, Permissions::from_mode(mode))
            .with_context(|| format!("Failed to chmod {}", path.display()))
            .map_err(CliError::unrunnable)
    }

    /// Returns whether a file or directory exists at `path`.
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    /// Recursively list all regular files under `dir`, returning absolute paths.
    fn list_files_under(&self, dir: &Path) -> CliResult<Vec<PathBuf>> {
        let mut out = Vec::new();
        walk_files(dir, &mut out)
            .with_context(|| format!("Failed to walk {}", dir.display()))
            .map_err(CliError::unrunnable)?;
        Ok(out)
    }
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
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
                Uuid::from_str(&id_var).context(format!(
                    "{EXECUTION_ID_ENV_VAR} must be a valid UUID: got {id_var:?}"
                ))
            })?;

        let execution_token = env::var(EXECUTION_TOKEN_ENV_VAR)
            .context(format!("{EXECUTION_TOKEN_ENV_VAR} must be set"))
            .and_then(|token_var| {
                Uuid::from_str(&token_var).context(format!(
                    "{EXECUTION_TOKEN_ENV_VAR} must be a valid UUID: got {token_var:?}"
                ))
            })?;

        let orchestrator_client =
            orchestrator::HttpClient::try_new(orchestrator_url, execution_id, execution_token)?;
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
    use crate::{
        kubernetes::mocks::MockClient as MockKubeClient,
        orchestrator::mocks::MockClient as MockOrchestrator,
    };
    use anyhow::anyhow;
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{Arc, RwLock},
    };

    /// In-memory filesystem state backing [`MockContext`]'s filesystem helpers. Shared via
    /// `Arc` so spawned tasks can observe writes made on the main test thread.
    #[derive(Default)]
    pub struct MockFs {
        pub files: RwLock<HashMap<PathBuf, Vec<u8>>>,
        pub permissions: RwLock<HashMap<PathBuf, u32>>,
    }

    pub struct MockContext {
        pub orchestrator_client: MockOrchestrator,
        pub kube_client: MockKubeClient,
        pub fs: Arc<MockFs>,
    }

    impl Default for MockContext {
        fn default() -> Self {
            Self {
                orchestrator_client: MockOrchestrator::default(),
                kube_client: MockKubeClient::default(),
                fs: Arc::new(MockFs::default()),
            }
        }
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

        fn read_file_to_string(&self, path: &Path) -> CliResult<String> {
            let bytes = self.read_file(path)?;
            String::from_utf8(bytes)
                .with_context(|| format!("Failed to read {} as UTF-8", path.display()))
                .map_err(CliError::unrunnable)
        }

        fn read_file(&self, path: &Path) -> CliResult<Vec<u8>> {
            self.fs
                .files
                .read()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| {
                    CliError::unrunnable(anyhow!(
                        "Failed to read {}: file not found in mock filesystem",
                        path.display()
                    ))
                })
        }

        fn write_file(&self, path: &Path, content: &[u8]) -> CliResult<()> {
            self.fs
                .files
                .write()
                .unwrap()
                .insert(path.to_owned(), content.to_vec());
            Ok(())
        }

        fn set_permissions_mode(&self, path: &Path, mode: u32) -> CliResult<()> {
            self.fs
                .permissions
                .write()
                .unwrap()
                .insert(path.to_owned(), mode);
            Ok(())
        }

        fn path_exists(&self, path: &Path) -> bool {
            self.fs.files.read().unwrap().contains_key(path)
        }

        fn list_files_under(&self, dir: &Path) -> CliResult<Vec<PathBuf>> {
            let prefix = dir.to_owned();
            let mut paths: Vec<PathBuf> = self
                .fs
                .files
                .read()
                .unwrap()
                .keys()
                .filter(|p| p.starts_with(&prefix))
                .cloned()
                .collect();
            paths.sort();
            Ok(paths)
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
            // exit_status is deliberately dropped — non-terminal statuses must not carry one.
            assert!(updates[0].exit_status.is_none());
        });
    }

    #[tokio::test]
    async fn run_shell_includes_command_in_status_message() {
        let ctx = MockContext::default();
        ctx.run_shell(&mut Command::new("true"), Status::Provisioning)
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

    #[tokio::test]
    async fn run_shell_failed_command_maps_to_unrunnable() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Provisioning)
            .await
            .unwrap_err();

        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()));
    }

    #[tokio::test]
    async fn run_shell_error_message_includes_command() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(&mut Command::new("false"), Status::Provisioning)
            .await
            .unwrap_err();

        let msg = err.source_to_string();
        assert!(msg.contains("false"), "expected command in error: {msg}");
        assert!(
            msg.contains("Command failed"),
            "expected 'Command failed' in error: {msg}"
        );
    }

    #[tokio::test]
    async fn run_shell_returns_unrunnable_on_io_error() {
        let ctx = MockContext::default();
        let err = ctx
            .run_shell(
                &mut Command::new("this-binary-definitely-does-not-exist"),
                Status::Provisioning,
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
