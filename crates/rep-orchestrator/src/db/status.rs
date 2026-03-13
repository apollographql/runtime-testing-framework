use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use std::cmp::Ordering;

/// Status updates for test runs and executions are tracked as a time series, with the status of
/// the test run being driven by the statuses of the executions inside of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
pub struct StatusUpdate {
    pub status: Status,
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// An individual lifecycle status for a test run or execution.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, sqlx::Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(i32)]
pub enum Status {
    Initialising = 1,
    Provisioning = 2,
    Running = 3,
    Successful = 4,
    Failed = 5,
    Unrunnable = 6,
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
            (Initialising, _) | (_, Initialising) => Initialising,
        }
    }
}

impl PartialOrd for Status {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        use Status::*;

        let sort_val = |&s| match s {
            Initialising => 0,
            Provisioning => 1,
            Running => 2,
            Successful | Failed | Unrunnable => 3, // all count as "complete"
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
        &[Provisioning, Running, Unrunnable, Failed, Successful], &[Initialising], &[];
        "initialising"
    )]
    #[test_case(
        Provisioning,
        &[Running, Unrunnable, Failed, Successful], &[Provisioning], &[Initialising];
        "provisioning"
    )]
    #[test_case(
        Running,
        &[Unrunnable, Failed, Successful], &[Running], &[Initialising, Provisioning];
        "running"
    )]
    #[test_case(
        Unrunnable,
        &[], &[Unrunnable, Failed, Successful], &[Initialising, Provisioning, Running];
        "unrunnable"
    )]
    #[test_case(
        Failed,
        &[], &[Unrunnable, Failed, Successful], &[Initialising, Provisioning, Running];
        "failed"
    )]
    #[test_case(
        Successful,
        &[], &[Unrunnable, Failed, Successful], &[Initialising, Provisioning, Running];
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
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_matching_works(status: Status) {
        assert_eq!(status.combine(status), status);
    }

    #[test_case(Failed; "failed")]
    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Running; "running")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_successful_is_other(other: Status) {
        assert_eq!(Successful.combine(other), other, "successful + other");
        assert_eq!(other.combine(Successful), other, "other + successful");
    }

    #[test_case(Unrunnable; "unrunnable")]
    #[test_case(Running; "running")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_failed_is_failed(other: Status) {
        assert_eq!(Failed.combine(other), Failed, "failed + other");
        assert_eq!(other.combine(Failed), Failed, "other + failed");
    }

    #[test_case(Running; "running")]
    #[test_case(Provisioning; "provisioning")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_unrunnable(other: Status) {
        assert_eq!(Unrunnable.combine(other), Unrunnable, "unrunnable + other");
        assert_eq!(other.combine(Unrunnable), Unrunnable, "other + unrunnable");
    }

    #[test_case(Provisioning; "provisioning")]
    #[test_case(Initialising; "initialising")]
    #[test]
    fn combine_running(other: Status) {
        assert_eq!(Running.combine(other), Running, "running + other");
        assert_eq!(other.combine(Running), Running, "other + running");
    }

    #[test_case(Provisioning; "provisioning")]
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
