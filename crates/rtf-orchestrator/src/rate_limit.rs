//! Per-user rate limiting for triggering test runs.
//!
//! Admins and unauthenticated local users (only possible via requests from inside the orchestrator
//! pod itself) are exempt.
use crate::{
    Error,
    config::PerUserExecutionConfig,
    db::{ClusterId, TestRun},
    event_loop::EventQueueState,
    state::UserType,
};
use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::PgConnection;

#[derive(thiserror::Error, Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RateLimitError {
    #[error("{current}/{max} concurrent runs")]
    ConcurrentRuns { current: u64, max: u64 },

    #[error("{current}/{max} queued runs")]
    QueuedRuns { current: u64, max: u64 },

    #[error("{current}/{max} queued executions")]
    QueuedExecutions { current: u64, max: u64 },

    #[error("{current}/{max} runs in the last hour")]
    RunsPerHour { current: u64, max: u64 },
}

pub async fn apply_rate_limits(
    eq_state: &EventQueueState,
    per_user: &PerUserExecutionConfig,
    cluster: &ClusterId,
    user: &UserType,
    conn: &mut PgConnection,
) -> Result<usize, Error> {
    let identity = match user {
        UserType::User(_) | UserType::Automation { .. } => user.user_identity().unwrap(),
        UserType::Admin(_) | UserType::Unknown => return Ok(0),
    };

    let counts = eq_state.user_queue_counts(&identity, cluster).await;

    if counts.ongoing_runs >= per_user.max_concurrent_runs {
        return Err(Error::RateLimited {
            cluster: cluster.clone(),
            reason: RateLimitError::ConcurrentRuns {
                current: counts.ongoing_runs as u64,
                max: per_user.max_concurrent_runs as u64,
            },
        });
    } else if counts.queued_runs >= per_user.max_queued_runs {
        return Err(Error::RateLimited {
            cluster: cluster.clone(),
            reason: RateLimitError::QueuedRuns {
                current: counts.queued_runs as u64,
                max: per_user.max_queued_runs as u64,
            },
        });
    }

    let since = Utc::now() - Duration::hours(1);
    let started_in_last_hour =
        TestRun::started_since(&identity, cluster, since, conn).await? as usize;

    if started_in_last_hour >= per_user.max_runs_per_hour {
        return Err(Error::RateLimited {
            cluster: cluster.clone(),
            reason: RateLimitError::RunsPerHour {
                current: started_in_last_hour as u64,
                max: per_user.max_runs_per_hour as u64,
            },
        });
    }

    Ok(counts.queued_executions)
}

pub fn check_queued_executions(
    per_user: &PerUserExecutionConfig,
    cluster: &ClusterId,
    user: &UserType,
    current_queued_executions: usize,
    n_variants: usize,
) -> Result<(), Error> {
    if user.is_admin() {
        return Ok(());
    }

    let projected = current_queued_executions + n_variants;

    if projected > per_user.max_queued_executions {
        return Err(Error::RateLimited {
            cluster: cluster.clone(),
            reason: RateLimitError::QueuedExecutions {
                current: current_queued_executions as u64,
                max: per_user.max_queued_executions as u64,
            },
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    #[test]
    fn check_queued_executions_rejects_only_once_the_projected_total_exceeds_max() {
        let per_user = PerUserExecutionConfig {
            max_queued_executions: 5,
            ..Default::default()
        };
        let user = UserType::User("alice".to_string());

        let at_max = check_queued_executions(&per_user, &alpha_cluster(), &user, 3, 2);
        assert!(at_max.is_ok(), "{at_max:?}");

        let over_max = check_queued_executions(&per_user, &alpha_cluster(), &user, 4, 2);
        assert_matches!(
            over_max,
            Err(Error::RateLimited {
                reason: RateLimitError::QueuedExecutions { current: 4, max: 5 },
                ..
            }),
            "{over_max:?}"
        );
    }

    #[test]
    fn check_queued_executions_doesnt_apply_to_admins() {
        let per_user = PerUserExecutionConfig {
            max_queued_executions: 0,
            ..Default::default()
        };

        let res = check_queued_executions(
            &per_user,
            &alpha_cluster(),
            &UserType::Admin("alice".to_string()),
            100,
            100,
        );

        assert!(res.is_ok(), "{res:?}");
    }
}
