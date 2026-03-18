use crate::db::{Queryable, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Executor, FromRow, PgConnection};
use std::cmp::Ordering;

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
                sqlx::query(&format!(
                    "INSERT INTO {} (parent_id, message, status) VALUES ($1, $2, $3)",
                    Self::STATUS_TABLE
                ))
                .bind(self.id())
                .bind(message)
                .bind(status),
            )
            .await?;

            if status.is_complete() {
                conn.execute(
                    sqlx::query(&format!(
                        "UPDATE {} SET completed_at = NOW() WHERE id = $1;",
                        Self::TABLE_NAME
                    ))
                    .bind(self.id()),
                )
                .await?;
            }

            self.after_set_status(status, conn).await
        }
    }

    fn current_status(
        &self,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<StatusUpdate>> + Send {
        async move {
            Ok(sqlx::query_as(&format!(
                "SELECT status, message, updated_at
                 FROM {}
                 WHERE parent_id = $1
                 ORDER BY updated_at DESC
                 LIMIT 1;",
                Self::STATUS_TABLE
            ))
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
            Ok(sqlx::query_as(&format!(
                "SELECT status, message, updated_at
                 FROM {}
                 WHERE parent_id = $1
                 ORDER BY updated_at DESC;",
                Self::STATUS_TABLE
            ))
            .bind(self.id())
            .fetch_all(conn)
            .await?)
        }
    }
}

/// Status updates for test runs and executions are tracked as a time series, with the status of
/// the test run being driven by the statuses of the executions inside of it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize, FromRow)]
pub struct StatusUpdate {
    pub status: Status,
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// An individual lifecycle status for a test run or execution.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Deserialize, Serialize, sqlx::Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(i32)]
pub enum Status {
    #[default]
    Initialising = 1,
    Resolving = 2,
    Provisioning = 3,
    Running = 4,
    Successful = 5,
    Failed = 6,
    Unrunnable = 7,
}

impl Status {
    /// Whether or not this status represents a terminal state.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Successful | Self::Failed | Self::Unrunnable)
    }

    /// Combine two statuses together to determine the overall status of a parent.
    pub fn combine(self, other: Status) -> Status {
        use Status::*;

        match (self, other) {
            (Successful, Successful) => Successful,
            (Failed, _) | (_, Failed) => Failed,
            (Unrunnable, _) | (_, Unrunnable) => Unrunnable,
            (Running, _) | (_, Running) => Running,
            (Provisioning, _) | (_, Provisioning) => Provisioning,
            (Resolving, _) | (_, Resolving) => Resolving,
            (Initialising, _) | (_, Initialising) => Initialising,
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
            Running => 3,
            Successful | Failed | Unrunnable => 4, // all count as "complete"
        };

        sort_val(self).partial_cmp(&sort_val(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::*;
    use simple_test_case::test_case;

    #[test_case(
        Initialising,
        &[Resolving, Provisioning, Running, Unrunnable, Failed, Successful], &[Initialising], &[];
        "initialising"
    )]
    #[test_case(
        Resolving,
        &[Provisioning, Running, Unrunnable, Failed, Successful], &[Resolving], &[Initialising];
        "resolving"
    )]
    #[test_case(
        Provisioning,
        &[Running, Unrunnable, Failed, Successful], &[Provisioning], &[Initialising, Resolving];
        "provisioning"
    )]
    #[test_case(
        Running,
        &[Unrunnable, Failed, Successful], &[Running], &[Initialising, Resolving, Provisioning];
        "running"
    )]
    #[test_case(
        Unrunnable,
        &[], &[Unrunnable, Failed, Successful], &[Initialising, Resolving, Provisioning, Running];
        "unrunnable"
    )]
    #[test_case(
        Failed,
        &[], &[Unrunnable, Failed, Successful], &[Initialising, Resolving, Provisioning, Running];
        "failed"
    )]
    #[test_case(
        Successful,
        &[], &[Unrunnable, Failed, Successful], &[Initialising, Resolving, Provisioning, Running];
        "successful"
    )]
    #[test]
    fn status_ordering_works(s: Status, lt: &[Status], eq: &[Status], gt: &[Status]) {
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
    #[test_case(Running; "running")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_matching_works(status: Status) {
        assert_eq!(status.combine(status), status);
    }

    #[test_case(Failed; "failed")]
    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Running; "running")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_successful_is_other(other: Status) {
        assert_eq!(Successful.combine(other), other, "successful + other");
        assert_eq!(other.combine(Successful), other, "other + successful");
    }

    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Running; "running")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_failed_is_failed(other: Status) {
        assert_eq!(Failed.combine(other), Failed, "failed + other");
        assert_eq!(other.combine(Failed), Failed, "other + failed");
    }

    #[test_case(Running; "running")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_unrunnable(other: Status) {
        assert_eq!(Unrunnable.combine(other), Unrunnable, "unrunnable + other");
        assert_eq!(other.combine(Unrunnable), Unrunnable, "other + unrunnable");
    }

    #[test_case(Provisioning; "provisioning")]
    #[test_case(Resolving; "resolving")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_running(other: Status) {
        assert_eq!(Running.combine(other), Running, "running + other");
        assert_eq!(other.combine(Running), Running, "other + running");
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
    fn combine_init_init_works() {
        assert_eq!(Initialising.combine(Initialising), Initialising)
    }
}
