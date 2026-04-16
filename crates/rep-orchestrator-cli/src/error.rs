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

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;
    use std::os::unix::process::ExitStatusExt;

    fn exit_code(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    #[derive(Debug)]
    enum ExpectedError {
        Unrunnable,
        UnrunnableSubprocess(i32),
        Failed(i32),
    }

    fn build(scenario: ExpectedError, msg: &str) -> CliError {
        match scenario {
            ExpectedError::Unrunnable => CliError::unrunnable(anyhow!("{}", msg)),
            ExpectedError::UnrunnableSubprocess(code) => {
                CliError::unrunnable_subprocess(exit_code(code), msg.to_owned())
            }
            ExpectedError::Failed(code) => CliError::failed(exit_code(code), msg.to_owned()),
        }
    }

    #[test_case(ExpectedError::Unrunnable, "oops", None, Status::Unrunnable; "unrunnable has no exit status")]
    #[test_case(ExpectedError::UnrunnableSubprocess(1), "cmd failed", None, Status::Unrunnable; "unrunnable subprocess suppresses exit status")]
    #[test_case(ExpectedError::Failed(1), "test failed", Some(1), Status::Failed; "failed maps to failed status")]
    #[test]
    fn error_properties(
        constructor: ExpectedError,
        msg: &str,
        expected_exit_code: Option<i32>,
        expected_status: Status,
    ) {
        let err = build(constructor, msg);
        assert_eq!(err.exit_status().and_then(|s| s.code()), expected_exit_code);
        assert_eq!(err.rep_orchestrator_status(), expected_status);
    }

    #[test_case(ExpectedError::Unrunnable, "something broke", "something broke"; "unrunnable")]
    #[test_case(ExpectedError::Failed(1), "tests failed", "tests failed"; "failed")]
    #[test]
    fn source_to_string_returns_message(constructor: ExpectedError, msg: &str, expected: &str) {
        let err = build(constructor, msg);
        assert_eq!(err.source_to_string(), expected);
    }

    #[test_case(ExpectedError::Unrunnable, "something broke", "something broke"; "unrunnable without exit code")]
    #[test_case(ExpectedError::UnrunnableSubprocess(1), "cmd failed", "(1) cmd failed"; "unrunnable subprocess includes exit code")]
    #[test_case(ExpectedError::Failed(2), "tests failed", "(2) tests failed"; "failed includes exit code")]
    #[test]
    fn display_formatting(constructor: ExpectedError, msg: &str, expected: &str) {
        let err = build(constructor, msg);
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn err_unrunnable_converts_anyhow_to_unrunnable() {
        let result: Result<(), anyhow::Error> = Err(anyhow!("bad thing"));
        let cli_err = result.map_err(CliError::unrunnable).unwrap_err();
        assert_eq!(cli_err.rep_orchestrator_status(), Status::Unrunnable);
        assert_eq!(cli_err.exit_status(), None);
        assert_eq!(cli_err.source_to_string(), "bad thing");
    }

    #[test]
    fn err_unrunnable_passes_through_ok() {
        let result: Result<u32, anyhow::Error> = Ok(42);
        assert_eq!(result.map_err(CliError::unrunnable).unwrap(), 42);
    }
}
