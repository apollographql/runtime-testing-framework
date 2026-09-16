use crate::db::{Error, Queryable, Result};
use chrono::{DateTime, Utc};
use rtf_orchestrator_shared::status::{Status as SharedStatus, StatusUpdate as SharedStatusUpdate};
use sqlx::{AssertSqlSafe, Executor, FromRow, PgConnection};
use std::{cmp::Ordering, fmt};

/// Helper trait for tracking a time series of [StatusUpdate] items for a parent table.
///
/// # Primary table requirements
/// - Must contain a nullable timestamp "completed_at" column
///
/// # Status table structure
/// This trait requires a fixed structure for the status table:
/// - integer "parent_id"
/// - integer "status"
/// - nullable text "message"
/// - timestamp "updated_at"
///
/// # Semantics
/// See the documentation on the [Status] enum for how each of the different statuses are used.
pub trait StatusTracked: Queryable {
    const STATUS_TABLE: &'static str;

    /// Additional logic to run after recording a status update for this type.
    fn after_set_status(
        &self,
        status: Status,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<()>> + Send;

    fn set_status(
        &self,
        status: Status,
        message: Option<String>,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            conn.execute(
                sqlx::query(AssertSqlSafe(format!(
                    "INSERT INTO {} (parent_id, message, status) VALUES ($1, $2, $3)",
                    Self::STATUS_TABLE
                )))
                .bind(self.id())
                .bind(message)
                .bind(status),
            )
            .await?;

            if status.is_terminal() {
                conn.execute(
                    sqlx::query(AssertSqlSafe(format!(
                        "UPDATE {} SET completed_at = NOW() WHERE id = $1;",
                        Self::TABLE_NAME
                    )))
                    .bind(self.id()),
                )
                .await?;
            }

            self.after_set_status(status, conn).await
        }
    }

    fn try_current_status(
        &self,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<Option<StatusUpdate>>> + Send {
        async move {
            Ok(sqlx::query_as(AssertSqlSafe(format!(
                "SELECT status, message, updated_at
                 FROM {}
                 WHERE parent_id = $1
                 ORDER BY updated_at DESC
                 LIMIT 1;",
                Self::STATUS_TABLE
            )))
            .bind(self.id())
            .fetch_optional(conn)
            .await?)
        }
    }

    fn current_status(
        &self,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<StatusUpdate>> + Send {
        async move {
            Ok(sqlx::query_as(AssertSqlSafe(format!(
                "SELECT status, message, updated_at
                 FROM {}
                 WHERE parent_id = $1
                 ORDER BY updated_at DESC
                 LIMIT 1;",
                Self::STATUS_TABLE
            )))
            .bind(self.id())
            .fetch_one(conn)
            .await?)
        }
    }

    fn status_history(
        &self,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<Vec<StatusUpdate>>> + Send {
        async move {
            Ok(sqlx::query_as(AssertSqlSafe(format!(
                "SELECT status, message, updated_at
                 FROM {}
                 WHERE parent_id = $1
                 ORDER BY updated_at DESC;",
                Self::STATUS_TABLE
            )))
            .bind(self.id())
            .fetch_all(conn)
            .await?)
        }
    }
}

/// Status updates for test runs and executions are tracked as a time series, with the status of
/// the test run being driven by the statuses of the executions inside of it.
#[derive(Debug, Default, Clone, PartialEq, Eq, FromRow)]
pub struct StatusUpdate {
    pub(crate) status: Status,
    pub(crate) message: Option<String>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<StatusUpdate> for SharedStatusUpdate {
    fn from(u: StatusUpdate) -> Self {
        Self {
            status: u.status.into(),
            message: u.message,
            updated_at: u.updated_at,
        }
    }
}

/// An individual lifecycle status for a test run or execution.
///
/// We enforce that status updates only over move forward through the ordering shown in the enum
/// definition here. Other than with with the three terminal statuses, it is possible for a
/// [StatusUpdate] to be created with a status that matches the current value. This is to allow for
/// fine grain messages to be recorded without needing to add a variant per operation.
///
///
/// # Test Run statuses vs Test Execution status
/// The majority of status updates made are against Test Executions rather than Test Runs. Other
/// than their original `Initialising` status, Test Runs receive their status updates via a "high
/// watermark" mechanism through status updates submitted against their Test Executions. See the
/// `status_after_execution` method for details of the semantics of how this mechanism works.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, sqlx::Type)]
#[repr(i32)]
pub enum Status {
    /// Initialising denotes that this entity has been acknowledged by the server but that no
    /// further action has been taken yet other than creating the initial database entry.
    #[default]
    Initialising = 1,

    /// Resolving denotes that the central resolver task has picked up this entity and is in the
    /// process of resolving the associated RTF test plan and carrying out validation checks.
    Resolving = 2,

    /// Provisioning denotes that we are in the process of creating the per-execution namespace and
    /// associated resources that are needed to process a given execution. We are deliberately
    /// verbose with the number of Provisioning updates we make to allow users to follow the
    /// progress of their workloads as they run.
    Provisioning = 3,

    /// EnvironmentReady denotes that the Argo workflow responsible for creating the
    /// per-execution namespace has completed successfully and that we are ready to trigger the
    /// scenario job.
    EnvironmentReady = 4,

    /// Running denotes that the RTF scenario has pulled all of the resources it needs and is now
    /// being run. This is set immediately prior to invoking the user provided scenario command.
    Running = 5,

    /// Successful denotes receiving a 0 exit code from the user provided RTF scenario command and
    /// is one of the three terminal states for a status tracked entity.
    Successful = 6,

    /// Failed denotes receiving a non-0 exit code from the user provided RTF scenario command and
    /// is one of the three terminal states for a status tracked entity.
    Failed = 7,

    /// Unrunnable denotes encountering a non-recoverable error during the process of running a
    /// given test execution. This covers all internal errors within the orchestrator and sidecar
    /// container as well as any errors that arise from being unable to successfully provision the
    /// RTF environment or scenario (such as docker images not being available or containers not
    /// reaching a ready status in the cluster).
    Unrunnable = 8,

    /// Cancelled denotes the run being prematurely cancelled before it ran to completion.
    Cancelled = 9,
}

impl Status {
    /// Whether or not this status represents a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Successful | Self::Failed | Self::Unrunnable | Self::Cancelled
        )
    }

    /// Combine two statuses together to determine the overall status of a parent.
    pub fn combine(self, other: Status) -> Status {
        use Status::*;

        match (self, other) {
            (Successful, Successful) => Successful,
            (Failed, _) | (_, Failed) => Failed,
            (Unrunnable, _) | (_, Unrunnable) => Unrunnable,
            (Cancelled, _) | (_, Cancelled) => Cancelled,
            (Running, _) | (_, Running) => Running,
            (EnvironmentReady, _) | (_, EnvironmentReady) => EnvironmentReady,
            (Provisioning, _) | (_, Provisioning) => Provisioning,
            (Resolving, _) | (_, Resolving) => Resolving,
            (Initialising, _) | (_, Initialising) => Initialising,
        }
    }

    pub fn validate_update(&self, new: Status, exit_code: Option<u8>) -> Result<()> {
        use Status::*;

        // We check strictly greater than in order to allow multiple updates at the same Status
        // with different messages (e.g. the different stages of provisioning). But we disallow
        // moving backward through the statuses or setting multiple terminal statuses.
        if *self > new || self.is_terminal() {
            return Err(Error::InvalidExecutionStatus {
                current: *self,
                requested: new,
            });
        }

        match (new, exit_code) {
            (Failed, None) => return Err(Error::MissingExitCode),
            (Failed, Some(0)) => return Err(Error::InvalidFailedExitCode),
            (Failed, Some(_)) => (),
            (Successful, Some(0)) => (),
            (_, Some(code)) => {
                return Err(Error::InvalidExitCode { status: new, code });
            }
            _ => (),
        };

        Ok(())
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Status::*;

        match self {
            Initialising => write!(f, "INITIALISING"),
            Resolving => write!(f, "RESOLVING"),
            Provisioning => write!(f, "PROVISIONING"),
            EnvironmentReady => write!(f, "ENVIRONMENT_READY"),
            Running => write!(f, "RUNNING"),
            Successful => write!(f, "SUCCESSFUL"),
            Failed => write!(f, "FAILED"),
            Unrunnable => write!(f, "UNRUNNABLE"),
            Cancelled => write!(f, "CANCELLED"),
        }
    }
}

impl PartialOrd for Status {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        use Status::*;

        let sort_val = |&s| match s {
            Initialising => 0,
            Resolving => 1,
            Provisioning => 2,
            EnvironmentReady => 3,
            Running => 4,
            Successful | Failed | Unrunnable | Cancelled => 5, // all count as "complete"
        };

        sort_val(self).partial_cmp(&sort_val(other))
    }
}

impl From<Status> for SharedStatus {
    fn from(s: Status) -> Self {
        use Status::*;

        match s {
            Initialising => Self::Initialising,
            Resolving => Self::Resolving,
            Provisioning => Self::Provisioning,
            EnvironmentReady => Self::EnvironmentReady,
            Running => Self::Running,
            Successful => Self::Successful,
            Failed => Self::Failed,
            Unrunnable => Self::Unrunnable,
            Cancelled => Self::Cancelled,
        }
    }
}

impl From<SharedStatus> for Status {
    fn from(s: SharedStatus) -> Self {
        use SharedStatus::*;

        match s {
            Initialising => Self::Initialising,
            Resolving => Self::Resolving,
            Provisioning => Self::Provisioning,
            EnvironmentReady => Self::EnvironmentReady,
            Running => Self::Running,
            Successful => Self::Successful,
            Failed => Self::Failed,
            Unrunnable => Self::Unrunnable,
            Cancelled => Self::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::*;
    use simple_test_case::test_case;
    use std::assert_matches;

    #[test_case(
        Initialising,
        &[Resolving, Provisioning, EnvironmentReady, Running, Cancelled, Unrunnable, Failed, Successful], &[Initialising], &[];
        "initialising"
    )]
    #[test_case(
        Resolving,
        &[Provisioning, EnvironmentReady, Running, Cancelled, Unrunnable, Failed, Successful], &[Resolving], &[Initialising];
        "resolving"
    )]
    #[test_case(
        Provisioning,
        &[EnvironmentReady, Running, Cancelled, Unrunnable, Failed, Successful], &[Provisioning], &[Initialising, Resolving];
        "provisioning"
    )]
    #[test_case(
        EnvironmentReady,
        &[Running, Cancelled, Unrunnable, Failed, Successful], &[EnvironmentReady], &[Initialising, Resolving, Provisioning];
        "environment_ready"
    )]
    #[test_case(
        Running,
        &[Cancelled, Unrunnable, Failed, Successful], &[Running], &[Initialising, Resolving, Provisioning, EnvironmentReady];
        "running"
    )]
    #[test_case(
        Cancelled,
        &[], &[Cancelled, Unrunnable, Failed, Successful], &[Initialising, Resolving, Provisioning, EnvironmentReady, Running];
        "cancelled"
    )]
    #[test_case(
        Unrunnable,
        &[], &[Cancelled, Unrunnable, Failed, Successful], &[Initialising, Resolving, Provisioning, EnvironmentReady, Running];
        "unrunnable"
    )]
    #[test_case(
        Failed,
        &[], &[Cancelled, Unrunnable, Failed, Successful], &[Initialising, Resolving, Provisioning, EnvironmentReady, Running];
        "failed"
    )]
    #[test_case(
        Successful,
        &[], &[Cancelled, Unrunnable, Failed, Successful], &[Initialising, Resolving, Provisioning, EnvironmentReady, Running];
        "successful"
    )]
    #[test]
    fn partial_cmp_returns_correct_ordering(
        s: Status,
        lt: &[Status],
        eq: &[Status],
        gt: &[Status],
    ) {
        for other in lt.iter() {
            assert_eq!(s.partial_cmp(other), Some(Ordering::Less), "{other:?}");
        }
        for other in eq.iter() {
            assert_eq!(s.partial_cmp(other), Some(Ordering::Equal), "{other:?}");
        }
        for other in gt.iter() {
            assert_eq!(s.partial_cmp(other), Some(Ordering::Greater), "{other:?}");
        }
    }

    #[test_case(Successful; "successful")]
    #[test_case(Failed; "failed")]
    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Cancelled; "cancelled")]
    #[test_case(Running; "running")]
    #[test_case(EnvironmentReady; "environment_ready")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_matching_statuses_returns_same_status(status: Status) {
        assert_eq!(status.combine(status), status);
    }

    #[test_case(Failed; "failed")]
    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Cancelled; "cancelled")]
    #[test_case(Running; "running")]
    #[test_case(EnvironmentReady; "environment_ready")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_successful_is_other(other: Status) {
        assert_eq!(Successful.combine(other), other, "successful + other");
        assert_eq!(other.combine(Successful), other, "other + successful");
    }

    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Cancelled; "cancelled")]
    #[test_case(Running; "running")]
    #[test_case(EnvironmentReady; "environment_ready")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_failed_is_failed(other: Status) {
        assert_eq!(Failed.combine(other), Failed, "failed + other");
        assert_eq!(other.combine(Failed), Failed, "other + failed");
    }

    #[test_case(Cancelled; "cancelled")]
    #[test_case(Running; "running")]
    #[test_case(EnvironmentReady; "environment_ready")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_unrunnable_is_unrunnable(other: Status) {
        assert_eq!(Unrunnable.combine(other), Unrunnable, "unrunnable + other");
        assert_eq!(other.combine(Unrunnable), Unrunnable, "other + unrunnable");
    }

    #[test_case(Running; "running")]
    #[test_case(EnvironmentReady; "environment_ready")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_cancelled_is_cancelled(other: Status) {
        assert_eq!(Cancelled.combine(other), Cancelled, "cancelled + other");
        assert_eq!(other.combine(Cancelled), Cancelled, "other + cancelled");
    }

    #[test_case(EnvironmentReady; "environment_ready")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_running_is_running(other: Status) {
        assert_eq!(Running.combine(other), Running, "running + other");
        assert_eq!(other.combine(Running), Running, "other + running");
    }

    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_environment_ready_is_environment_ready(other: Status) {
        assert_eq!(
            EnvironmentReady.combine(other),
            EnvironmentReady,
            "env_ready + other"
        );
        assert_eq!(
            other.combine(EnvironmentReady),
            EnvironmentReady,
            "other + env_ready"
        );
    }

    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_provisioning(other: Status) {
        assert_eq!(Provisioning.combine(other), Provisioning, "prov + other");
        assert_eq!(other.combine(Provisioning), Provisioning, "other + prov");
    }

    #[test]
    fn combine_init_init_returns_initialising() {
        assert_eq!(Initialising.combine(Initialising), Initialising)
    }

    // valid
    #[test_case(EnvironmentReady, None, Ok(()); "valid environment_ready from provisioning")]
    #[test_case(Running, None, Ok(()); "valid non-error")]
    #[test_case(Provisioning, None, Ok(()); "valid repeat of current status")]
    #[test_case(Successful, Some(0), Ok(()); "valid successful with 0 exit code")]
    #[test_case(Failed, Some(1), Ok(()); "valid error")]
    // invalid
    #[test_case(Failed, Some(0), Err(Error::InvalidFailedExitCode); "error with 0 exit code")]
    #[test_case(Failed, None, Err(Error::MissingExitCode); "error without exit code")]
    #[test_case(Running, Some(0), Err(Error::InvalidExitCode { status: Running, code: 0 }); "unexpected exit code")]
    #[test_case(
        Initialising,
        None,
        Err(Error::InvalidExecutionStatus { current: Provisioning, requested: Initialising });
        "status rollback"
    )]
    #[test]
    fn validation_works(status: Status, exit_code: Option<u8>, expected: Result<()>) {
        let res = Provisioning.validate_update(status, exit_code);

        match (expected, res) {
            (Ok(()), Ok(())) => (),
            (Err(e1), Err(e2)) if e1.to_string() == e2.to_string() => (),
            (r1, r2) => panic!("expected {r1:?}, got {r2:?}"),
        }
    }

    #[test_case(Successful; "successful")]
    #[test_case(Failed; "failed")]
    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Cancelled; "cancelled")]
    #[test]
    fn validate_second_terminal_status_is_invalid(current: Status) {
        let res = current.validate_update(Successful, None);

        assert_matches!(
            res,
            Err(Error::InvalidExecutionStatus {
                current: _,
                requested: Successful
            }),
            "{res:?}"
        );
    }
}
