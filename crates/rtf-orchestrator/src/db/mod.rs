use rtf_orchestrator_shared::payload::PreparedPayload;
use serde::Serialize;
use sqlx::{Database, FromRow, PgConnection, Postgres};
use std::fmt;
use thiserror::Error;
use tracing::error;
use uuid::Uuid;

mod known_test_plan;
pub mod pool;
mod status;
mod test_execution;
pub mod test_plan_history;
mod test_run;
mod test_run_filter;
mod variables;

pub use known_test_plan::{KnownTestPlan, KnownTestPlanFilter, KnownTestPlanRun};
pub use status::{Status, StatusTracked, StatusUpdate};
pub use test_execution::TestExecution;
pub use test_run::TestRun;
pub use test_run_filter::TestRunFilter;
pub use variables::upsert_variables;

#[macro_export]
macro_rules! conn {
    { } => {
        &mut *(
            $crate::db::pool::get_pool()
                .await?
                .acquire()
                .await
                .map_err($crate::db::Error::from)?
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    #[error("requested execution status ({requested}) does not follow current status ({current})")]
    InvalidExecutionStatus { current: Status, requested: Status },

    #[error("non-terminal status updates may not include a status code")]
    InvalidExitCode { status: Status, code: u8 },

    #[error("FAILED status updates must have a non-zero exit code")]
    InvalidFailedExitCode,

    #[error("FAILED status updates must include an exit code")]
    MissingExitCode,

    #[error("a known test plan with this name, or this org/repo/path, is already registered")]
    KnownTestPlanAlreadyExists,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ClusterId(String);

impl ClusterId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClusterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Helper trait for common queries and semantics when interacting with the DB.
///
/// # Table requirements
/// - Must contain an integer "id" column
pub trait Queryable:
    Send + Sync + Unpin + for<'r> FromRow<'r, <Postgres as Database>::Row>
{
    const TABLE_NAME: &'static str;

    fn id(&self) -> i32;

    fn get_by_id(
        id: i32,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<Option<Self>>> + Send {
        async move {
            Ok(sqlx::query_as(&format!(
                "SELECT * FROM {} WHERE id = $1;",
                Self::TABLE_NAME
            ))
            .bind(id)
            .fetch_optional(conn)
            .await?)
        }
    }

    fn get_by_id_unchecked(
        id: i32,
        conn: &mut PgConnection,
    ) -> impl Future<Output = Result<Self>> + Send {
        async move {
            Ok(sqlx::query_as(&format!(
                "SELECT * FROM {} WHERE id = $1;",
                Self::TABLE_NAME
            ))
            .bind(id)
            .fetch_one(conn)
            .await?)
        }
    }
}

pub trait UpdateHandle: Send + Sync {
    fn init_execution(
        &mut self,
        tr: &TestRun,
        name: &str,
        index: usize,
    ) -> impl Future<Output = crate::Result<TestExecution>> + Send;

    fn executions_for_run(
        &mut self,
        tr: &TestRun,
    ) -> impl Future<Output = crate::Result<Vec<TestExecution>>>;

    fn try_current_test_execution_status(
        &mut self,
        ex: &TestExecution,
    ) -> impl Future<Output = crate::Result<Option<StatusUpdate>>> + Send;

    fn update_test_run_status(
        &mut self,
        tr: &TestRun,
        status: Status,
        message: Option<String>,
    ) -> impl Future<Output = crate::Result<()>> + Send;

    fn update_test_execution_status(
        &mut self,
        ex: &TestExecution,
        status: Status,
        message: Option<String>,
    ) -> impl Future<Output = crate::Result<()>> + Send;

    fn mark_run_as_resolving(
        &mut self,
        tr: &TestRun,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_run_status(tr, Status::Resolving, Some(message))
                .await
            {
                error!(id=%tr.uuid(), %err, "Unable to mark Test Run as resolving");
            }
        }
    }

    fn mark_run_as_unrunnable(
        &mut self,
        tr: &TestRun,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_run_status(tr, Status::Unrunnable, Some(message))
                .await
            {
                error!(id=%tr.uuid(), %err, "Unable to mark Test Run as unrunnable");
            }
        }
    }

    fn mark_run_as_cancelled(
        &mut self,
        tr: &TestRun,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_run_status(tr, Status::Cancelled, Some(message))
                .await
            {
                error!(id=%tr.uuid(), %err, "Unable to mark Test Run as cancelled");
            }
        }
    }

    fn mark_execution_as_resolving(
        &mut self,
        ex: &TestExecution,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_execution_status(ex, Status::Resolving, Some(message))
                .await
            {
                error!(id=%ex.uuid(), %err, "Unable to mark Test Execution as resolving");
            }
        }
    }

    fn mark_execution_as_provisioning(
        &mut self,
        ex: &TestExecution,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_execution_status(ex, Status::Provisioning, Some(message))
                .await
            {
                error!(id=%ex.uuid(), %err, "Unable to mark Test Execution as provisioning");
            }
        }
    }

    fn mark_execution_as_environment_ready(
        &mut self,
        ex: &TestExecution,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_execution_status(ex, Status::EnvironmentReady, Some(message))
                .await
            {
                error!(id=%ex.uuid(), %err, "Unable to mark Test Execution as environment_ready");
            }
        }
    }

    fn mark_execution_as_unrunnable(
        &mut self,
        ex: &TestExecution,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_execution_status(ex, Status::Unrunnable, Some(message))
                .await
            {
                error!(id=%ex.uuid(), %err, "Unable to mark Test Execution as unrunnable");
            }
        }
    }

    fn mark_execution_as_cancelled(
        &mut self,
        ex: &TestExecution,
        message: String,
    ) -> impl Future<Output = ()> + Send {
        async {
            if let Err(err) = self
                .update_test_execution_status(ex, Status::Cancelled, Some(message))
                .await
            {
                error!(id=%ex.uuid(), %err, "Unable to mark Test Execution as cancelled");
            }
        }
    }

    fn cache_payload_for_run(
        &mut self,
        tr: &TestRun,
        payload: &PreparedPayload,
    ) -> impl Future<Output = ()> + Send;

    fn clear_cached_payload_for_run(&mut self, run_uuid: Uuid) -> impl Future<Output = ()> + Send;
}

impl UpdateHandle for PgConnection {
    async fn init_execution(
        &mut self,
        tr: &TestRun,
        name: &str,
        index: usize,
    ) -> crate::Result<TestExecution> {
        Ok(tr.init_execution(name, index, self).await?)
    }

    async fn executions_for_run(&mut self, tr: &TestRun) -> crate::Result<Vec<TestExecution>> {
        Ok(tr.executions(self).await?)
    }

    async fn try_current_test_execution_status(
        &mut self,
        ex: &TestExecution,
    ) -> crate::Result<Option<StatusUpdate>> {
        Ok(ex.try_current_status(self).await?)
    }

    async fn update_test_run_status(
        &mut self,
        tr: &TestRun,
        status: Status,
        message: Option<String>,
    ) -> crate::Result<()> {
        if let Some(current) = tr.try_current_status(self).await? {
            current.status.validate_update(status, None)?;
        }

        Ok(tr.set_status(status, message, self).await?)
    }

    async fn update_test_execution_status(
        &mut self,
        ex: &TestExecution,
        status: Status,
        message: Option<String>,
    ) -> crate::Result<()> {
        if let Some(current) = ex.try_current_status(self).await? {
            current.status.validate_update(status, None)?;
        }

        Ok(ex.set_status(status, message, self).await?)
    }

    async fn cache_payload_for_run(&mut self, tr: &TestRun, payload: &PreparedPayload) {
        if let Err(err) = tr.cache_payload(payload, self).await {
            error!(run_uuid=%tr.uuid(), %err, "Unable to cache payload for run");
        }
    }

    async fn clear_cached_payload_for_run(&mut self, run_uuid: Uuid) {
        if let Err(err) = TestRun::clear_cached_payload(run_uuid, self).await {
            error!(%run_uuid, %err, "Unable to evict cached payload for run");
        }
    }
}

#[cfg(test)]
pub use update_handle::{MockUpdateHandle, TaggedStatusUpdate};

#[cfg(test)]
mod update_handle {
    use super::*;
    use crate::Error;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum TaggedStatusUpdate {
        Run(i32, StatusUpdate),
        Execution(i32, StatusUpdate),
    }

    impl Default for TaggedStatusUpdate {
        fn default() -> Self {
            TaggedStatusUpdate::Run(-1, Default::default())
        }
    }

    impl TaggedStatusUpdate {
        pub fn run(id: i32, status: Status, message: Option<impl Into<String>>) -> Self {
            Self::Run(
                id,
                StatusUpdate {
                    status,
                    message: message.map(Into::into),
                    ..Default::default()
                },
            )
        }

        pub fn execution(id: i32, status: Status, message: Option<impl Into<String>>) -> Self {
            Self::Execution(
                id,
                StatusUpdate {
                    status,
                    message: message.map(Into::into),
                    ..Default::default()
                },
            )
        }
    }

    #[derive(Debug, Default)]
    pub struct MockUpdateHandle {
        pub test_runs: Vec<TestRun>,
        pub test_executions: Vec<TestExecution>,
        pub status_updates: Vec<TaggedStatusUpdate>,
        pub cached_payloads: Vec<Uuid>,
        pub cleared_payload_caches: Vec<Uuid>,
    }

    impl MockUpdateHandle {
        pub fn with_run(tr: TestRun) -> Self {
            Self {
                test_runs: vec![tr],
                ..Default::default()
            }
        }

        pub fn with_execution(ex: TestExecution) -> Self {
            Self {
                test_executions: vec![ex],
                ..Default::default()
            }
        }

        pub fn statuses(&self) -> Vec<Status> {
            self.status_updates
                .iter()
                .map(|u| match u {
                    TaggedStatusUpdate::Run(_, s) => s.status,
                    TaggedStatusUpdate::Execution(_, s) => s.status,
                })
                .collect()
        }
    }

    impl UpdateHandle for MockUpdateHandle {
        async fn init_execution(
            &mut self,
            tr: &TestRun,
            name: &str,
            index: usize,
        ) -> crate::Result<TestExecution> {
            if self.test_runs.iter().all(|elem| elem.id() != tr.id()) {
                return Err(Error::UnknownTestRun { id: tr.uuid() });
            }

            let ex =
                TestExecution::create_stub(self.test_executions.len() as i32, tr.id(), index, name);
            self.test_executions.push(ex.clone());

            Ok(ex)
        }

        async fn executions_for_run(&mut self, tr: &TestRun) -> crate::Result<Vec<TestExecution>> {
            Ok(self
                .test_executions
                .iter()
                .filter(|ex| ex.test_run_id() == tr.id())
                .cloned()
                .collect())
        }

        async fn try_current_test_execution_status(
            &mut self,
            ex: &TestExecution,
        ) -> crate::Result<Option<StatusUpdate>> {
            Ok(self
                .status_updates
                .iter()
                .rev()
                .filter_map(|u| match u {
                    TaggedStatusUpdate::Execution(id, s) if *id == ex.id() => Some(s.clone()),
                    _ => None,
                })
                .next())
        }

        async fn update_test_run_status(
            &mut self,
            tr: &TestRun,
            status: Status,
            message: Option<String>,
        ) -> crate::Result<()> {
            if self.test_runs.iter().all(|elem| elem.id() != tr.id()) {
                return Err(Error::UnknownTestRun { id: tr.uuid() });
            }

            self.status_updates
                .push(TaggedStatusUpdate::run(tr.id(), status, message));

            Ok(())
        }

        async fn update_test_execution_status(
            &mut self,
            ex: &TestExecution,
            status: Status,
            message: Option<String>,
        ) -> crate::Result<()> {
            if self.test_executions.iter().all(|elem| elem.id() != ex.id()) {
                return Err(Error::UnknownTestExecution { id: ex.uuid() });
            }

            self.status_updates
                .push(TaggedStatusUpdate::execution(ex.id(), status, message));

            Ok(())
        }

        async fn cache_payload_for_run(&mut self, tr: &TestRun, _payload: &PreparedPayload) {
            self.cached_payloads.push(tr.uuid());
        }

        async fn clear_cached_payload_for_run(&mut self, run_uuid: Uuid) {
            self.cleared_payload_caches.push(run_uuid);
        }
    }
}
