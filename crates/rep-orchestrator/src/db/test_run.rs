use crate::db::{
    self, Queryable, Result,
    status::{Status, StatusTracked, StatusUpdate},
    test_execution::TestExecution,
};
use chrono::{DateTime, Utc};
use rep_orchestrator_shared::summary::TestRunSummary;
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

/// A `TestRun` denotes a single user-triggered set of tests that should be considered passing or
/// failing based on their combined status.
///
/// This is a logical construct that we use to make it easier to trigger and query the results of
/// workloads submitted to the orchestrator. Each [TestRun] contains one or more [TestExecution]s
/// which denote the acutal tests being run.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TestRun {
    id: i32,
    uuid: Uuid,
    name: String,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

impl Queryable for TestRun {
    const TABLE_NAME: &'static str = "test_run";

    fn id(&self) -> i32 {
        self.id
    }
}

impl StatusTracked for TestRun {
    const STATUS_TABLE: &'static str = "test_run_status";

    // No additional logic needed when recording status items
    async fn after_set_status(&self, _status: Status, _conn: &mut PgConnection) -> Result<()> {
        Ok(())
    }
}

impl TestRun {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    #[cfg(test)]
    pub fn create_stub(id: i32, name: &str) -> Self {
        Self {
            id,
            uuid: Uuid::new_v4(),
            name: name.into(),
            started_at: Utc::now(),
            completed_at: None,
        }
    }

    pub async fn get_by_uuid(uuid: &Uuid, conn: &mut PgConnection) -> Result<Option<Self>> {
        Ok(sqlx::query_as("SELECT * FROM test_run WHERE uuid = $1;")
            .bind(uuid)
            .fetch_optional(conn)
            .await?)
    }

    pub async fn init(name: &str, conn: &mut PgConnection) -> Result<Self> {
        let tr: TestRun = sqlx::query_as(
            "INSERT INTO test_run (name, started_at)
             VALUES ($1, NOW())
             RETURNING id, uuid, name, started_at, completed_at;
            ",
        )
        .bind(name)
        .fetch_one(&mut *conn)
        .await?;

        tr.set_status(Status::Initialising, None, conn).await?;

        Ok(tr)
    }

    pub async fn init_execution(
        &self,
        name: &str,
        conn: &mut PgConnection,
    ) -> Result<TestExecution> {
        TestExecution::init(name, self.id, conn).await
    }

    pub async fn executions(&self, conn: &mut PgConnection) -> Result<Vec<TestExecution>> {
        Ok(
            sqlx::query_as("SELECT * FROM test_execution WHERE test_run_id = $1;")
                .bind(self.id)
                .fetch_all(conn)
                .await?,
        )
    }

    /// Following a status update for a child [TestExecution] we need to determine whether or not
    /// the overall status of this [TestRun] needs to be updated. This method will return
    /// `Some(status)` if the given `execution_status` triggers an update for the run as a whole,
    /// otherwise `None`.
    pub(super) async fn status_after_execution_update(
        &self,
        execution_status: Status,
        conn: &mut PgConnection,
    ) -> Result<Option<Status>> {
        let StatusUpdate {
            status: run_status, ..
        } = self.current_status(conn).await?;

        status_after_execution_update(run_status, execution_status, async move || {
            let sibling_executions = self.executions(conn).await?;
            let mut execution_statuses = Vec::with_capacity(sibling_executions.len());
            for ex in sibling_executions.iter() {
                execution_statuses.push(ex.current_status(conn).await?.status);
            }

            Ok(execution_statuses)
        })
        .await
    }

    pub async fn try_into_summary(self, conn: &mut PgConnection) -> Result<TestRunSummary> {
        let status_history = self.status_history(conn).await?;
        let current = self.current_status(conn).await?;
        let raw_executions = self.executions(conn).await?;

        let mut executions = Vec::with_capacity(raw_executions.len());
        for ex in raw_executions.into_iter() {
            // It is possible for us to encounter TestExecutions that are part way through
            // initialising when calling this method (as in, the row for the execution exists
            // in the DB but we don't yet have the first status row). Building a summary requires
            // at least one status, so we skip executions missing that information.
            //
            // The user facing effect of this is the same as if the main execution row had
            // not yet been created, namely that the execution is not yet present in the list
            match ex.try_into_summary(conn).await {
                Ok(summary) => executions.push(summary),
                Err(db::Error::Sqlx(sqlx::Error::RowNotFound)) => continue,
                Err(e) => return Err(e),
            }
        }

        Ok(TestRunSummary {
            id: self.uuid,
            name: self.name,
            current_status: current.status.into(),
            started_at: self.started_at,
            updated_at: current.updated_at,
            completed_at: self.completed_at,
            status_history: status_history.into_iter().map(Into::into).collect(),
            executions,
        })
    }
}

async fn status_after_execution_update<F, Fut>(
    run_status: Status,
    execution_status: Status,
    get_sibling_statuses: F,
) -> Result<Option<Status>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<Status>>>,
{
    use Status::*;

    let new_run_status = match (run_status, execution_status) {
        // Once at least one execution reports resolving, the run as a whole is resolving
        (Initialising, Resolving) => Some(Resolving),

        // Once at least one execution reports provisioning, the run as a whole is provisioning
        (Initialising | Resolving, Provisioning) => Some(Provisioning),

        // Once at least one execution reports running, the run as a whole is running
        (Initialising | Resolving | Provisioning, Running) => Some(Running),

        // Once all executions are complete we can determine the terminal status of the run.
        // Once the run has a terminal status, further updates are ignored
        (s_run, s_ex) if s_ex.is_complete() && !s_run.is_complete() => {
            let execution_statuses = (get_sibling_statuses)().await?;

            // Combine the statuses of all executions in this run. If the result is a terminal
            // status then we need to update.
            execution_statuses
                .into_iter()
                .reduce(|l, r| l.combine(r))
                .and_then(|s| if s.is_complete() { Some(s) } else { None })
        }

        _ => None,
    };

    Ok(new_run_status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conn,
        db::status::{Status, StatusTracked},
    };
    use Status::*;
    use simple_test_case::test_case;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_creates_run_with_initialising_status() -> Result<()> {
        let c = conn!();
        let res = TestRun::init("test", c).await;
        assert!(res.is_ok(), "{res:?}");

        let tr = res.unwrap();
        assert_eq!(tr.name, "test", "{tr:?}");

        let current = tr.current_status(c).await?;
        assert_eq!(current.status, Status::Initialising);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_id_returns_matching_run() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init("test", c).await?;
        let tr2 = TestRun::get_by_id(tr1.id, c).await?;

        assert_eq!(Some(tr1), tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_id_unchecked_returns_matching_run() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init("test", c).await?;
        let tr2 = TestRun::get_by_id_unchecked(tr1.id, c).await?;

        assert_eq!(tr1, tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_uuid_returns_matching_run() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init("test", c).await?;
        let tr2 = TestRun::get_by_uuid(&tr1.uuid, c).await?;

        assert_eq!(Some(tr1), tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn executions_returns_all_associated_executions() -> Result<()> {
        let c = conn!();

        let tr = TestRun::init("A", c).await?;
        let ex1 = tr.init_execution("a", c).await?;
        let ex2 = tr.init_execution("b", c).await?;

        let executions = tr.executions(c).await?;
        assert_eq!(executions.len(), 2, "wrong number of executions");
        assert_eq!(executions[0], ex1, "execution 1");
        assert_eq!(executions[1], ex2, "execution 2");

        Ok(())
    }

    // Status of Initialising is checked in `init_creates_run_with_initialising_status` above
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Resolving; "resolving")]
    #[test_case(Status::Provisioning; "provisioning")]
    #[test_case(Status::Running; "running")]
    #[test_case(Status::Successful; "successful")]
    #[test_case(Status::Failed; "failed")]
    #[test_case(Status::Unrunnable; "unrunnable")]
    #[tokio::test]
    async fn set_status_and_current_status_match(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;

        tr.set_status(status, None, c).await?;
        let current = tr.current_status(c).await?;

        assert_eq!(current.status, status);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn status_history_returns_entries_newest_first() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?; // sets Status::Initialising
        tr.set_status(Status::Running, None, c).await?;
        tr.set_status(Status::Successful, None, c).await?;

        let history = tr.status_history(c).await?;
        assert_eq!(history.len(), 3, "wrong number of history entries");
        assert_eq!(history[0].status, Status::Successful, "newest");
        assert_eq!(history[1].status, Status::Running, "second");
        assert_eq!(history[2].status, Status::Initialising, "oldest");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Successful; "successful")]
    #[test_case(Status::Failed; "failed")]
    #[test_case(Status::Unrunnable; "unrunnable")]
    #[tokio::test]
    async fn set_terminal_status_sets_completed_at(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        assert!(tr.completed_at.is_none());

        tr.set_status(status, None, c).await?;
        let tr = TestRun::get_by_id_unchecked(tr.id(), c).await?;
        assert!(tr.completed_at.is_some());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Initialising; "initialising")]
    #[test_case(Status::Resolving; "resolving")]
    #[test_case(Status::Provisioning; "provisioning")]
    #[test_case(Status::Running; "running")]
    #[tokio::test]
    async fn set_non_terminal_status_does_not_set_completed_at(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        assert!(tr.completed_at.is_none());

        tr.set_status(status, None, c).await?;
        let tr = TestRun::get_by_id_unchecked(tr.id(), c).await?;
        assert!(tr.completed_at.is_none());

        Ok(())
    }

    // First execution to hit Resolving/Provisioning/Running should update
    #[test_case(Initialising, Resolving, &[], Some(Resolving); "init to resolving")]
    #[test_case(Initialising, Provisioning, &[], Some(Provisioning); "init to provisioning")]
    #[test_case(Initialising, Running, &[], Some(Running); "init to running")]
    #[test_case(Resolving, Provisioning, &[], Some(Provisioning); "resolving to provisioning")]
    #[test_case(Resolving, Running, &[], Some(Running); "resolving to running")]
    #[test_case(Provisioning, Running, &[], Some(Running); "provisioning to running")]
    // Moving to Resolving/Provisioning/Running should only happen once
    #[test_case(Resolving, Resolving, &[], None; "already resolving")]
    #[test_case(Provisioning, Provisioning, &[], None; "already provisioning")]
    #[test_case(Running, Running, &[], None; "already running")]
    // Successful while siblings are ongoing
    #[test_case(Running, Successful, &[Running], None; "successful sibling running")]
    // Non-successful terminal while siblings are ongoing
    #[test_case(Running, Failed, &[Running, Successful], Some(Failed); "failed sibling running")]
    #[test_case(Running, Unrunnable, &[Running, Successful], Some(Unrunnable); "unrunnable sibling running")]
    // Last execution reporting terminal status
    #[test_case(Running, Successful, &[Successful], Some(Successful); "final successful")]
    #[test_case(Running, Unrunnable, &[Successful], Some(Unrunnable); "final unrunnable")]
    #[test_case(Running, Failed, &[Successful], Some(Failed); "final failed")]
    // Failed and Unrunnable eagerly update run status, so further terminal execution updates
    // should be ignored
    #[test_case(Failed, Successful, &[Failed], None; "final successful but already failed")]
    #[test_case(Unrunnable, Successful, &[Unrunnable], None; "final successful but already unrunnable")]
    #[tokio::test]
    async fn status_after_execution_update(
        run_status: Status,
        ex_status: Status,
        siblings: &[Status],
        expected: Option<Status>,
    ) -> Result<()> {
        let mut ex_statuses = siblings.to_vec();
        ex_statuses.push(ex_status);

        let new_status =
            status_after_execution_update(run_status, ex_status, async move || Ok(ex_statuses))
                .await?;

        assert_eq!(new_status, expected);

        Ok(())
    }
}
