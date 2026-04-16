use crate::db::{
    Queryable, Result,
    status::{Status, StatusTracked},
    test_run::TestRun,
};
use chrono::{DateTime, Utc};
use rep_orchestrator_shared::{
    status::StatusUpdate as SharedStatusUpdate, summary::TestExecutionSummary,
};
use sqlx::{Executor, FromRow, PgConnection};
use uuid::Uuid;

/// An individual `TestExecution` has the same semantics as a single RTF matrix variant and is our
/// unit of execution under the orchestrator.
///
/// Each execution represents the running of a user provided RTF scenario inside of an ephemeral
/// namespace within the workload cluster and is associated with a single parent [TestRun].
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TestExecution {
    id: i32,
    uuid: Uuid,
    test_run_id: i32,
    name: String,
    token: Uuid,
    exit_code: Option<i32>,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    has_file_upload: bool,
}

impl Queryable for TestExecution {
    const TABLE_NAME: &'static str = "test_execution";

    fn id(&self) -> i32 {
        self.id
    }
}

impl StatusTracked for TestExecution {
    const STATUS_TABLE: &'static str = "test_execution_status";

    async fn after_set_status(&self, status: Status, conn: &mut PgConnection) -> Result<()> {
        let tr = self.test_run(conn).await?;
        if let Some(new_status) = tr.status_after_execution_update(status, conn).await? {
            tr.set_status(new_status, None, conn).await?;
        };

        Ok(())
    }
}

impl TestExecution {
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn token(&self) -> &Uuid {
        &self.token
    }

    pub fn has_file_upload(&self) -> bool {
        self.has_file_upload
    }

    pub fn is_complete(&self) -> bool {
        self.completed_at.is_some()
    }

    pub fn log_file_gcs_object_name(&self) -> String {
        format!("{}/log.txt", self.uuid)
    }

    pub fn output_zip_gcs_object_name(&self) -> String {
        format!("{}/output.zip", self.uuid)
    }

    #[cfg(test)]
    pub fn create_stub(id: i32, test_run_id: i32, name: &str) -> Self {
        Self {
            id,
            uuid: Uuid::new_v4(),
            test_run_id,
            name: name.into(),
            token: Uuid::new_v4(),
            exit_code: None,
            started_at: Utc::now(),
            completed_at: None,
            has_file_upload: false,
        }
    }

    pub async fn get_by_uuid(uuid: &Uuid, conn: &mut PgConnection) -> Result<Option<Self>> {
        Ok(
            sqlx::query_as("SELECT * FROM test_execution WHERE uuid = $1;")
                .bind(uuid)
                .fetch_optional(conn)
                .await?,
        )
    }

    pub async fn init(name: &str, test_run_id: i32, conn: &mut PgConnection) -> Result<Self> {
        let ex: TestExecution = sqlx::query_as(
            "INSERT INTO test_execution (test_run_id, name)
             VALUES ($1, $2)
             RETURNING id, uuid, test_run_id, name, token, exit_code, started_at, completed_at, has_file_upload;
            ",
        )
        .bind(test_run_id)
        .bind(name)
        .fetch_one(&mut *conn)
        .await?;

        ex.set_status(Status::Initialising, None, conn).await?;

        Ok(ex)
    }

    pub async fn set_exit_code(&mut self, code: u8, conn: &mut PgConnection) -> Result<()> {
        conn.execute(
            sqlx::query("UPDATE test_execution SET exit_code = $1 WHERE id = $2;")
                .bind(code as i32)
                .bind(self.id),
        )
        .await?;

        self.exit_code = Some(code as i32);

        Ok(())
    }

    pub async fn mark_has_file_upload(&mut self, conn: &mut PgConnection) -> Result<()> {
        conn.execute(
            sqlx::query("UPDATE test_execution SET has_file_upload = true WHERE id = $1;")
                .bind(self.id),
        )
        .await?;

        self.has_file_upload = true;

        Ok(())
    }

    pub async fn test_run(&self, conn: &mut PgConnection) -> Result<TestRun> {
        TestRun::get_by_id_unchecked(self.test_run_id, conn).await
    }

    pub async fn try_into_summary(self, conn: &mut PgConnection) -> Result<TestExecutionSummary> {
        let status_history = self.status_history(conn).await?;
        let current: SharedStatusUpdate = self.current_status(conn).await?.into();

        Ok(TestExecutionSummary {
            id: self.uuid,
            name: self.name,
            current_status: current.status,
            exit_code: self.exit_code,
            started_at: self.started_at,
            updated_at: current.updated_at,
            completed_at: self.completed_at,
            status_history: status_history.into_iter().map(Into::into).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conn,
        db::status::{Status, StatusTracked},
    };
    use simple_test_case::test_case;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_creates_execution_with_initialising_status() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let res = TestExecution::init("test", tr.id(), c).await;
        assert!(res.is_ok(), "{res:?}");

        let ex = res.unwrap();
        assert_eq!(ex.name, "test", "{ex:?}");
        assert_eq!(ex.test_run_id, tr.id(), "{ex:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_id_returns_matching_execution() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex1 = TestExecution::init("test", tr.id(), c).await?;
        let ex2 = TestExecution::get_by_id(ex1.id, c).await?;

        assert_eq!(Some(ex1), ex2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_id_unchecked_returns_matching_execution() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex1 = TestExecution::init("test", tr.id(), c).await?;
        let ex2 = TestExecution::get_by_id_unchecked(ex1.id, c).await?;

        assert_eq!(ex1, ex2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_uuid_returns_matching_execution() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex1 = TestExecution::init("test", tr.id(), c).await?;
        let ex2 = TestExecution::get_by_uuid(&ex1.uuid, c).await?;

        assert_eq!(Some(ex1), ex2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn test_run_returns_parent_run() -> Result<()> {
        let c = conn!();

        let tr = TestRun::init("A", c).await?;
        let ex1 = TestExecution::init("a", tr.id(), c).await?;
        let ex2 = TestExecution::init("b", tr.id(), c).await?;

        let tr_a = ex1.test_run(c).await?;
        assert_eq!(tr_a, tr, "execution 1");

        let tr_b = ex2.test_run(c).await?;
        assert_eq!(tr_b, tr, "execution 2");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn set_exit_code_persists_value() -> Result<()> {
        let c = conn!();

        let tr = TestRun::init("A", c).await?;
        let mut ex1 = TestExecution::init("a", tr.id(), c).await?;

        assert!(ex1.exit_code.is_none(), "after init: {ex1:?}");

        ex1.set_exit_code(42, c).await?;
        assert_eq!(ex1.exit_code, Some(42), "updated struct: {ex1:?}");

        let queried = TestExecution::get_by_id_unchecked(ex1.id, c).await?;
        assert_eq!(queried.exit_code, Some(42), "queried struct: {queried:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn mark_has_file_upload_persists_value() -> Result<()> {
        let c = conn!();

        let tr = TestRun::init("A", c).await?;
        let mut ex1 = TestExecution::init("a", tr.id(), c).await?;

        assert!(!ex1.has_file_upload, "after init: {ex1:?}");

        ex1.mark_has_file_upload(c).await?;
        assert!(ex1.has_file_upload, "updated struct: {ex1:?}");

        let queried = TestExecution::get_by_id_unchecked(ex1.id, c).await?;
        assert!(queried.has_file_upload, "queried struct: {queried:?}");

        Ok(())
    }

    // Status of Initialising is checked in `init_creates_execution_with_initialising_status` above
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Provisioning; "provisioning")]
    #[test_case(Status::Running; "running")]
    #[test_case(Status::Successful; "successful")]
    #[test_case(Status::Failed; "failed")]
    #[test_case(Status::Unrunnable; "unrunnable")]
    #[tokio::test]
    async fn set_status_and_current_status_match(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex = TestExecution::init("test", tr.id(), c).await?;

        ex.set_status(status, None, c).await?;
        let current = ex.current_status(c).await?;

        assert_eq!(current.status, status);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn status_history_returns_entries_newest_first() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex = TestExecution::init("test", tr.id(), c).await?; // sets Status::Initialising
        ex.set_status(Status::Running, None, c).await?;
        ex.set_status(Status::Successful, None, c).await?;

        let history = ex.status_history(c).await?;
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
        let ex = TestExecution::init("test", tr.id(), c).await?;
        assert!(ex.completed_at.is_none());

        ex.set_status(status, None, c).await?;
        let ex = TestExecution::get_by_id_unchecked(ex.id(), c).await?;
        assert!(ex.completed_at.is_some());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Initialising; "initialising")]
    #[test_case(Status::Provisioning; "provisioning")]
    #[test_case(Status::Running; "running")]
    #[tokio::test]
    async fn set_non_terminal_status_does_not_set_completed_at(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex = TestExecution::init("test", tr.id(), c).await?;
        assert!(ex.completed_at.is_none());

        ex.set_status(status, None, c).await?;
        let ex = TestExecution::get_by_id_unchecked(ex.id(), c).await?;
        assert!(ex.completed_at.is_none());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(None, Status::Provisioning; "initialising to provisioning")]
    #[test_case(None, Status::Running; "initialising to running")]
    #[test_case(Some(Status::Provisioning), Status::Running; "provisioning to running")]
    #[tokio::test]
    async fn execution_provisioning_and_running_update_parent_run(
        run_status: Option<Status>,
        execution_status: Status,
    ) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        if let Some(s) = run_status {
            tr.set_status(s, None, c).await?;
        }

        let ex = tr.init_execution("test", c).await?;
        ex.set_status(execution_status, None, c).await?;

        assert_eq!(tr.current_status(c).await?.status, execution_status);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Provisioning; "provisioning")]
    #[test_case(Status::Running; "running")]
    #[tokio::test]
    async fn execution_provisioning_and_running_are_high_water_mark(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        tr.set_status(status, None, c).await?;
        let ex = tr.init_execution("test", c).await?;

        let history_before = tr.status_history(c).await?;
        ex.set_status(status, None, c).await?;
        let history_after = tr.status_history(c).await?;

        // We were already in the correct status so there shouldn't be a further status update
        assert_eq!(
            history_before, history_after,
            "no new status entry expected"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Successful; "successful")]
    #[test_case(Status::Failed; "failed")]
    #[test_case(Status::Unrunnable; "unrunnable")]
    #[tokio::test]
    async fn single_execution_terminal_status_propagates_immediately(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex = tr.init_execution("test", c).await?;
        ex.set_status(status, None, c).await?;

        let current = tr.current_status(c).await?;
        assert_eq!(current.status, status);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn terminal_status_propagation_requires_all_executions() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init("test", c).await?;
        let ex1 = tr.init_execution("a", c).await?;
        let ex2 = tr.init_execution("b", c).await?;
        let ex3 = tr.init_execution("c", c).await?;

        ex1.set_status(Status::Successful, None, c).await?;
        let current = tr.current_status(c).await?;
        let is_complete = current.status.is_complete();
        assert!(!is_complete, "after ex1: {current:?}");

        ex2.set_status(Status::Successful, None, c).await?;
        let current = tr.current_status(c).await?;
        let is_complete = current.status.is_complete();
        assert!(!is_complete, "after ex2: {current:?}");

        ex3.set_status(Status::Successful, None, c).await?;
        let current = tr.current_status(c).await?;
        let is_complete = current.status.is_complete();
        assert!(is_complete, "after ex3: {current:?}");

        Ok(())
    }
}
