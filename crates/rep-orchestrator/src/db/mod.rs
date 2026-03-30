use sqlx::{Database, FromRow, PgConnection, Postgres};
use thiserror::Error;
use tracing::error;

pub mod pool;
mod status;
mod test_execution;
mod test_run;

pub use status::{Status, StatusTracked, StatusUpdate};
pub use test_execution::TestExecution;
pub use test_run::TestRun;

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
    ) -> impl Future<Output = crate::Result<TestExecution>> + Send;

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
}

impl UpdateHandle for PgConnection {
    async fn init_execution(&mut self, tr: &TestRun, name: &str) -> crate::Result<TestExecution> {
        Ok(tr.init_execution(name, self).await?)
    }

    async fn update_test_run_status(
        &mut self,
        tr: &TestRun,
        status: Status,
        message: Option<String>,
    ) -> crate::Result<()> {
        Ok(tr.set_status(status, message, self).await?)
    }

    async fn update_test_execution_status(
        &mut self,
        ex: &TestExecution,
        status: Status,
        message: Option<String>,
    ) -> crate::Result<()> {
        Ok(ex.set_status(status, message, self).await?)
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
    }

    impl MockUpdateHandle {
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
        ) -> crate::Result<TestExecution> {
            if self.test_runs.iter().all(|elem| elem.id() != tr.id()) {
                return Err(Error::UnknownTestRun { id: tr.uuid() });
            }

            let ex = TestExecution::create_stub(self.test_executions.len() as i32, tr.id(), name);
            self.test_executions.push(ex.clone());

            Ok(ex)
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
    }
}
