use crate::{kubernetes, orchestrator};
use anyhow::Context;
use rep_orchestrator_shared::{
    EXECUTION_ID_ENV_VAR, EXECUTION_TOKEN_ENV_VAR, ORCHESTRATOR_URL_ENV_VAR,
};
use std::{
    env, fmt,
    fs::{self, Permissions, set_permissions},
    io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    str::FromStr,
};
use tracing::info;
use uuid::Uuid;

/// Describes which filesystem operation failed, used by [FsError] to compose its display message.
#[derive(Debug)]
pub enum FsErrorKind {
    Read,
    Write,
    SetPermissions,
    CreateDir,
    ListFiles,
}

impl fmt::Display for FsErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => f.write_str("read"),
            Self::Write => f.write_str("write"),
            Self::SetPermissions => f.write_str("set permissions on"),
            Self::CreateDir => f.write_str("create directory"),
            Self::ListFiles => f.write_str("list files under"),
        }
    }
}

/// An error produced when reading, writing, or traversing the filesystem.
#[derive(Debug, thiserror::Error)]
#[error("failed to {kind} {path}")]
pub struct FsError {
    pub path: PathBuf,
    pub kind: FsErrorKind,
    #[source]
    pub source: io::Error,
}

/// Errors produced when invoking shell commands.
///
/// Implements [`Display`] manually so [`ShellError::Failed`] can conditionally
/// render the exit code (processes terminated by a signal have no code).
#[derive(Debug)]
pub enum ShellError {
    Spawn {
        cmd: String,
        source: io::Error,
    },

    Failed {
        cmd: String,
        exit_status: ExitStatus,
    },
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { cmd, .. } => write!(f, "I/O error when attempting to invoke {cmd}"),
            Self::Failed { cmd, exit_status } => match exit_status.code() {
                Some(code) => write!(f, "({code}) command failed: {cmd}"),
                None => write!(f, "command failed: {cmd}"),
            },
        }
    }
}

impl std::error::Error for ShellError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::Failed { .. } => None,
        }
    }
}

pub trait CliContext {
    type OrchestratorClient: orchestrator::Client;
    type KubeClient: kubernetes::Client;

    fn orchestrator_client(&self) -> &Self::OrchestratorClient;

    fn kube_client(&self) -> &Self::KubeClient;

    /// Run an orchestration shell [Command] to completion.
    ///
    /// Logs the command arguments before running. On failure, logs captured stderr and returns a
    /// [Status::Unrunnable] error — every caller is an orchestration subroutine, so there is no
    /// distinction between "setup failed" and "user scenario failed" here.
    fn run_shell(&self, cmd: &mut Command) -> impl Future<Output = crate::Result<()>> + Send;

    /// Read the file at `path` as UTF-8.
    fn read_file_to_string(&self, path: &Path) -> Result<String, FsError>;

    /// Read the file at `path` as bytes.
    fn read_file(&self, path: &Path) -> Result<Vec<u8>, FsError>;

    /// Write `content` to `path`.
    fn write_file(&self, path: &Path, content: &[u8]) -> Result<(), FsError>;

    /// chmod `path` to `mode`.
    fn set_permissions_mode(&self, path: &Path, mode: u32) -> Result<(), FsError>;

    /// Returns whether a file or directory exists at `path`.
    fn path_exists(&self, path: &Path) -> bool;

    /// Recursively list all regular files under `dir`, returning absolute paths.
    fn list_files_under(&self, dir: &Path) -> Result<Vec<PathBuf>, FsError>;
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

    async fn run_shell(&self, cmd: &mut Command) -> crate::Result<()> {
        run_shell(cmd).await
    }

    fn read_file_to_string(&self, path: &Path) -> Result<String, FsError> {
        fs::read_to_string(path).map_err(|source| FsError {
            path: path.to_owned(),
            kind: FsErrorKind::Read,
            source,
        })
    }

    fn read_file(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        fs::read(path).map_err(|source| FsError {
            path: path.to_owned(),
            kind: FsErrorKind::Read,
            source,
        })
    }

    fn write_file(&self, path: &Path, content: &[u8]) -> Result<(), FsError> {
        fs::write(path, content).map_err(|source| FsError {
            path: path.to_owned(),
            kind: FsErrorKind::Write,
            source,
        })
    }

    fn set_permissions_mode(&self, path: &Path, mode: u32) -> Result<(), FsError> {
        set_permissions(path, Permissions::from_mode(mode)).map_err(|source| FsError {
            path: path.to_owned(),
            kind: FsErrorKind::SetPermissions,
            source,
        })
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn list_files_under(&self, dir: &Path) -> Result<Vec<PathBuf>, FsError> {
        let mut out = Vec::new();
        walk_files(dir, &mut out).map_err(|source| FsError {
            path: dir.to_owned(),
            kind: FsErrorKind::ListFiles,
            source,
        })?;

        Ok(out)
    }
}

async fn run_shell(cmd: &mut Command) -> crate::Result<()> {
    info!("running {cmd:?}");

    let exit_status = cmd.status().map_err(|source| ShellError::Spawn {
        cmd: format!("{cmd:?}"),
        source,
    })?;

    if !exit_status.success() {
        return Err(ShellError::Failed {
            cmd: format!("{cmd:?}"),
            exit_status,
        }
        .into());
    }

    info!("command succeeded: {cmd:?}");

    Ok(())
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

#[cfg(test)]
pub(crate) mod mocks {
    use super::*;
    use crate::{
        kubernetes::mocks::MockClient as MockKubeClient,
        orchestrator::mocks::MockClient as MockOrchestrator,
    };
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

        async fn run_shell(&self, cmd: &mut Command) -> crate::Result<()> {
            run_shell(cmd).await
        }

        fn read_file_to_string(&self, path: &Path) -> Result<String, FsError> {
            let bytes = self.read_file(path)?;
            String::from_utf8(bytes).map_err(|_| FsError {
                path: path.to_owned(),
                kind: FsErrorKind::Read,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "file is not valid UTF-8",
                ),
            })
        }

        fn read_file(&self, path: &Path) -> Result<Vec<u8>, FsError> {
            self.fs
                .files
                .read()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| FsError {
                    path: path.to_owned(),
                    kind: FsErrorKind::Read,
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "file not found in mock filesystem",
                    ),
                })
        }

        fn write_file(&self, path: &Path, content: &[u8]) -> Result<(), FsError> {
            self.fs
                .files
                .write()
                .unwrap()
                .insert(path.to_owned(), content.to_vec());

            Ok(())
        }

        fn set_permissions_mode(&self, path: &Path, mode: u32) -> Result<(), FsError> {
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

        fn list_files_under(&self, dir: &Path) -> Result<Vec<PathBuf>, FsError> {
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
    use crate::context::mocks::MockContext;

    #[tokio::test]
    async fn run_shell_does_not_post_status_on_success() {
        let ctx = MockContext::default();
        ctx.run_shell(&mut Command::new("true")).await.unwrap();

        ctx.orchestrator_client()
            .read_updates(|updates| assert!(updates.is_empty()));
    }

    #[tokio::test]
    async fn run_shell_error_message_includes_command() {
        let ctx = MockContext::default();
        let err = ctx.run_shell(&mut Command::new("false")).await.unwrap_err();

        let msg = err.to_string();

        assert!(msg.contains("false"), "expected command in error: {msg}");
        assert!(
            msg.contains("command failed"),
            "expected 'command failed' in error: {msg}"
        );
    }
}
