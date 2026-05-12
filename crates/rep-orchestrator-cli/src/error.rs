use crate::{
    context::{FsError, ShellError},
    kubernetes, orchestrator,
};
use thiserror::Error;

/// All errors that prevent test execution from proceeding.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Kubernetes(#[from] kubernetes::Error),

    #[error(transparent)]
    OrchestratorApi(#[from] orchestrator::Error),

    #[error(transparent)]
    Filesystem(#[from] FsError),

    #[error(transparent)]
    Shell(#[from] ShellError),
}

pub type CliResult<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::FsErrorKind;
    use std::{io, path::PathBuf};

    fn filesystem_error() -> Error {
        FsError {
            path: PathBuf::from("/some/file"),
            kind: FsErrorKind::Read,
            source: io::Error::new(io::ErrorKind::NotFound, "not found"),
        }
        .into()
    }

    fn shell_spawn_error() -> Error {
        ShellError::Spawn {
            cmd: "my-cmd".to_owned(),
            source: io::Error::new(io::ErrorKind::NotFound, "not found"),
        }
        .into()
    }

    fn shell_failed_error(code: i32) -> Error {
        use std::{os::unix::process::ExitStatusExt, process::ExitStatus};
        ShellError::Failed {
            cmd: "my-cmd".to_owned(),
            exit_status: ExitStatus::from_raw(code << 8),
        }
        .into()
    }

    #[test]
    fn filesystem_error_message_includes_path() {
        let msg = filesystem_error().to_string();
        assert!(msg.contains("/some/file"), "expected path in error: {msg}");
    }

    #[test]
    fn shell_failed_error_message_includes_exit_code() {
        let msg = shell_failed_error(2).to_string();
        assert!(msg.contains("(2)"), "expected exit code in error: {msg}");
        assert!(msg.contains("my-cmd"), "expected cmd in error: {msg}");
    }

    #[test]
    fn shell_spawn_error_message_includes_cmd() {
        let msg = shell_spawn_error().to_string();
        assert!(msg.contains("my-cmd"), "expected cmd in spawn error: {msg}");
    }
}
