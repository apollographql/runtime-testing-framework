use crate::db::{Queryable, Result, test_execution::TestExecution};
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

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

impl TestRun {
    pub async fn get_by_uuid(uuid: &Uuid, conn: &mut PgConnection) -> Result<Option<Self>> {
        Ok(sqlx::query_as("SELECT * FROM test_run WHERE uuid = $1;")
            .bind(uuid)
            .fetch_optional(conn)
            .await?)
    }

    pub async fn create(name: &str, conn: &mut PgConnection) -> Result<Self> {
        let tr = sqlx::query_as(
            "INSERT INTO test_run (name, started_at)
             VALUES ($1, NOW())
             RETURNING id, uuid, name, started_at, completed_at;
            ",
        )
        .bind(name)
        .fetch_one(conn)
        .await?;

        Ok(tr)
    }

    pub async fn executions(&self, conn: &mut PgConnection) -> Result<Vec<TestExecution>> {
        Ok(
            sqlx::query_as("SELECT * FROM test_execution WHERE test_run_id = $1;")
                .bind(self.id)
                .fetch_all(conn)
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn create_works() -> Result<()> {
        let res = TestRun::create("test", conn!()).await;
        assert!(res.is_ok(), "{res:?}");

        let tr = res.unwrap();
        assert_eq!(tr.name, "test", "{tr:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn get_by_id_works() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::create("test", c).await?;
        let tr2 = TestRun::get_by_id(tr1.id, c).await?;

        assert_eq!(Some(tr1), tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn get_by_id_unchecked_works() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::create("test", c).await?;
        let tr2 = TestRun::get_by_id_unchecked(tr1.id, c).await?;

        assert_eq!(tr1, tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn get_by_uuid_works() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::create("test", c).await?;
        let tr2 = TestRun::get_by_uuid(&tr1.uuid, c).await?;

        assert_eq!(Some(tr1), tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn executions_works() -> Result<()> {
        let c = conn!();

        let tr = TestRun::create("A", c).await?;
        let ex1 = TestExecution::create("a", tr.id(), c).await?;
        let ex2 = TestExecution::create("b", tr.id(), c).await?;

        let executions = tr.executions(c).await?;
        assert_eq!(executions.len(), 2, "wrong number of executions");
        assert_eq!(executions[0], ex1, "execution 1");
        assert_eq!(executions[1], ex2, "execution 2");

        Ok(())
    }
}
