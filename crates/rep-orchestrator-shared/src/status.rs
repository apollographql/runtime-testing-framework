use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Status updates for test runs and executions are tracked as a time series, with the status of
/// the test run being driven by the statuses of the executions inside of it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct StatusUpdate {
    pub status: Status,
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// An individual lifecycle status for a test run or execution.
///
/// See the documentation on the sibling `Status` enum in the `rep_orchestrator` crate for details
/// on semantics.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    #[default]
    Initialising,
    Resolving,
    Provisioning,
    EnvironmentReady,
    Running,
    Successful,
    Failed,
    Unrunnable,
}

impl Status {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Successful | Self::Failed | Self::Unrunnable)
    }

    /// The coarse lifecycle bucket this status falls into, e.g. for presenting a handful of visual
    /// states (colours, icons) instead of every individual status.
    pub fn category(&self) -> StatusCategory {
        use Status::*;

        match self {
            Initialising | Resolving | Provisioning => StatusCategory::Pending,
            EnvironmentReady | Running => StatusCategory::Active,
            Successful => StatusCategory::Success,
            Failed | Unrunnable => StatusCategory::Failed,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Status::*;

        match self {
            Initialising => write!(f, "INITIALISING"),
            Resolving => write!(f, "RESOLVING"),
            Provisioning => write!(f, "PROVISIONING"),
            EnvironmentReady => write!(f, "ENV_READY"),
            Running => write!(f, "RUNNING"),
            Successful => write!(f, "SUCCESSFUL"),
            Failed => write!(f, "FAILED"),
            Unrunnable => write!(f, "UNRUNNABLE"),
        }
    }
}

/// A coarse grouping of [Status] values, for presentation contexts that distinguish only a handful
/// of visual states rather than every individual status.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum StatusCategory {
    Pending,
    Active,
    Success,
    Failed,
}
