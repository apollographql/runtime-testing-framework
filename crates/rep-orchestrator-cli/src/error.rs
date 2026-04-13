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

impl CliError {
    /// The exit status of the process that caused this error, if any
    pub fn exit_status(&self) -> Option<ExitStatus> {
        match self {
            // For the purposes of writing to the database, Unrunnable errors should not report an exit status directly
            Self::Unrunnable { exit_status, .. } => None,
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
