use anyhow::anyhow;
use rep_orchestrator_shared::status::Status;
use std::{
    fmt::{self, Display, Formatter},
    process::ExitStatus,
};
use thiserror::Error;

/// Errors produced within this CLI that map 1:1 with terminal failure statuses in the REP Orchestrator.
///
/// See the documentation of [Status] for details.
#[derive(Debug, Error)]
pub enum CliError {
    Unrunnable {
        source: anyhow::Error,
        /// Unrunnable errors may include an exit code for [Display] purposes, but this will not be recorded to the database
        exit_status: Option<ExitStatus>,
    },
    Failed {
        source: anyhow::Error,
        /// Failures in a user's test plan **must** have an associated exit code to record to the database
        exit_status: ExitStatus,
    },
}

pub type CliResult<T> = Result<T, CliError>;

pub trait ToCliResult<T> {
    fn err_unrunnable(self) -> CliResult<T>;
}

impl CliError {
    /// Construct a [CliError::Unrunnable] error from the provided `source` error
    pub fn unrunnable(source: anyhow::Error) -> Self {
        Self::Unrunnable {
            source,
            exit_status: None,
        }
    }

    /// Construct a [CliError::Unrunnable] error from the provided subprocess `exit_status` and additional `context`
    pub fn unrunnable_subprocess(exit_status: ExitStatus, context: String) -> Self {
        Self::Unrunnable {
            source: anyhow!(context),
            exit_status: Some(exit_status),
        }
    }

    /// Construct a [CliError::Failed] error from the provided subprocess `exit_status` and additional `context`
    pub fn failed(exit_status: ExitStatus, context: String) -> Self {
        Self::Failed {
            source: anyhow!(context),
            exit_status,
        }
    }

    /// The exit status of the process that caused this error, if any
    pub fn exit_status(&self) -> Option<ExitStatus> {
        match self {
            // For the purposes of writing to the database, Unrunnable errors should not report an exit status directly
            Self::Unrunnable { .. } => None,
            Self::Failed { exit_status, .. } => Some(*exit_status),
        }
    }

    /// The String form of the underlying error
    pub fn source_to_string(&self) -> String {
        match self {
            Self::Unrunnable { source, .. } | Self::Failed { source, .. } => source.to_string(),
        }
    }

    /// The [Status] that this variant maps to
    pub fn rep_orchestrator_status(&self) -> Status {
        match self {
            Self::Unrunnable { .. } => Status::Unrunnable,
            Self::Failed { .. } => Status::Failed,
        }
    }
}

fn format_with_exit_code(
    formatter: &mut Formatter,
    source: &anyhow::Error,
    code: Option<i32>,
) -> fmt::Result {
    match code {
        Some(code) => {
            write!(formatter, "({code}) {}", source)
        }
        None => source.fmt(formatter),
    }
}

impl Display for CliError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unrunnable {
                source,
                exit_status,
            } => format_with_exit_code(
                formatter,
                source,
                exit_status.and_then(|status| status.code()),
            ),
            Self::Failed {
                source,
                exit_status,
            } => format_with_exit_code(formatter, source, exit_status.code()),
        }
    }
}

impl<T> ToCliResult<T> for Result<T, anyhow::Error> {
    fn err_unrunnable(self) -> CliResult<T> {
        self.map_err(CliError::unrunnable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn exit_code(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    #[test]
    fn unrunnable_has_no_exit_status() {
        let err = CliError::unrunnable(anyhow!("oops"));
        assert_eq!(err.exit_status(), None);
    }

    #[test]
    fn unrunnable_maps_to_unrunnable_status() {
        let err = CliError::unrunnable(anyhow!("oops"));
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }

    #[test]
    fn unrunnable_subprocess_suppresses_exit_status() {
        let err = CliError::unrunnable_subprocess(exit_code(1), "cmd failed".to_owned());
        assert_eq!(err.exit_status(), None);
        assert_eq!(err.rep_orchestrator_status(), Status::Unrunnable);
    }

    #[test]
    fn failed_maps_to_failed_status() {
        let err = CliError::failed(exit_code(1), "test failed".to_owned());
        assert_eq!(err.rep_orchestrator_status(), Status::Failed);
    }

    #[test]
    fn source_to_string_returns_only_message_for_unrunnable() {
        let err = CliError::unrunnable(anyhow!("something broke"));
        assert_eq!(err.source_to_string(), "something broke");
    }

    #[test]
    fn source_to_string_returns_only_message_for_failed() {
        let err = CliError::failed(exit_code(1), "tests failed".to_owned());
        assert_eq!(err.source_to_string(), "tests failed");
    }

    #[test]
    fn display_unrunnable_without_exit_code_shows_message_only() {
        let err = CliError::unrunnable(anyhow!("something broke"));
        assert_eq!(format!("{err}"), "something broke");
    }

    #[test]
    fn display_unrunnable_subprocess_includes_exit_code() {
        let err = CliError::unrunnable_subprocess(exit_code(1), "cmd failed".to_owned());
        assert_eq!(format!("{err}"), "(1) cmd failed");
    }

    #[test]
    fn display_failed_includes_exit_code() {
        let err = CliError::failed(exit_code(2), "tests failed".to_owned());
        assert_eq!(format!("{err}"), "(2) tests failed");
    }

    #[test]
    fn err_unrunnable_converts_anyhow_to_unrunnable() {
        let result: Result<(), anyhow::Error> = Err(anyhow!("bad thing"));
        let cli_err = result.err_unrunnable().unwrap_err();
        assert_eq!(cli_err.rep_orchestrator_status(), Status::Unrunnable);
        assert_eq!(cli_err.exit_status(), None);
        assert_eq!(cli_err.source_to_string(), "bad thing");
    }

    #[test]
    fn err_unrunnable_passes_through_ok() {
        let result: Result<u32, anyhow::Error> = Ok(42);
        assert_eq!(result.err_unrunnable().unwrap(), 42);
    }
}
