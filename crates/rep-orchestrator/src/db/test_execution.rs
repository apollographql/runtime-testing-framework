use crate::db::{Queryable, Result, test_run::TestRun};
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TestExecution {
    id: i32,
    uuid: Uuid,
    test_run_id: i32,
    name: String,
    exit_code: Option<i32>,
    started_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

impl Queryable for TestExecution {
    const TABLE_NAME: &'static str = "test_execution";

    fn id(&self) -> i32 {
        self.id
    }
}

impl TestExecution {
    pub async fn get_by_uuid(uuid: &Uuid, conn: &mut PgConnection) -> Result<Option<Self>> {
        Ok(
            sqlx::query_as("SELECT * FROM test_execution WHERE uuid = $1;")
                .bind(uuid)
                .fetch_optional(conn)
                .await?,
        )
    }

    pub async fn create(name: &str, test_run_id: i32, conn: &mut PgConnection) -> Result<Self> {
        let ex = sqlx::query_as(
            "INSERT INTO test_execution (test_run_id, name)
             VALUES ($1, $2)
             RETURNING id, uuid, test_run_id, name, exit_code, started_at, updated_at, completed_at;
            ",
        )
        .bind(test_run_id)
        .bind(name)
        .fetch_one(conn)
        .await?;

        Ok(ex)
    }

    pub async fn test_run(&self, conn: &mut PgConnection) -> Result<TestRun> {
        TestRun::get_by_id_unchecked(self.test_run_id, conn).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn create_works() -> Result<()> {
        let c = conn!();
        let tr = TestRun::create("test", c).await?;
        let res = TestExecution::create("test", tr.id(), c).await;
        assert!(res.is_ok(), "{res:?}");

        let ex = res.unwrap();
        assert_eq!(ex.name, "test", "{ex:?}");
        assert_eq!(ex.test_run_id, tr.id(), "{ex:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn get_by_id_works() -> Result<()> {
        let c = conn!();
        let tr = TestRun::create("test", c).await?;
        let ex1 = TestExecution::create("test", tr.id(), c).await?;
        let ex2 = TestExecution::get_by_id(ex1.id, c).await?;

        assert_eq!(Some(ex1), ex2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn get_by_id_unchecked_works() -> Result<()> {
        let c = conn!();
        let tr = TestRun::create("test", c).await?;
        let ex1 = TestExecution::create("test", tr.id(), c).await?;
        let ex2 = TestExecution::get_by_id_unchecked(ex1.id, c).await?;

        assert_eq!(ex1, ex2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn get_by_uuid_works() -> Result<()> {
        let c = conn!();
        let tr = TestRun::create("test", c).await?;
        let ex1 = TestExecution::create("test", tr.id(), c).await?;
        let ex2 = TestExecution::get_by_uuid(&ex1.uuid, c).await?;

        assert_eq!(Some(ex1), ex2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_run_works() -> Result<()> {
        let c = conn!();

        let tr = TestRun::create("A", c).await?;
        let ex1 = TestExecution::create("a", tr.id(), c).await?;
        let ex2 = TestExecution::create("b", tr.id(), c).await?;

        let tr_a = ex1.test_run(c).await?;
        assert_eq!(tr_a, tr, "execution 1");

        let tr_b = ex2.test_run(c).await?;
        assert_eq!(tr_b, tr, "execution 2");

        Ok(())
    }
}
