use crate::db::{Result, TestRun};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Postgres, QueryBuilder};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestRunFilter {
    pub name: Option<String>,
    pub initiated_by: Option<String>,
    pub started_after: Option<DateTime<Utc>>,
    pub started_before: Option<DateTime<Utc>>,
}

impl TestRunFilter {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.initiated_by.is_none()
            && self.started_after.is_none()
            && self.started_before.is_none()
    }

    fn push_where_clause(&self, qb: &mut QueryBuilder<'_, Postgres>) {
        if self.is_empty() {
            return;
        }

        qb.push(" WHERE TRUE");

        if let Some(name) = &self.name {
            qb.push(" AND name = ").push_bind(name.clone());
        }
        if let Some(user) = &self.initiated_by {
            qb.push(" AND initiated_by = ").push_bind(user.clone());
        }
        if let Some(dt) = self.started_after {
            qb.push(" AND started_at >= ").push_bind(dt);
        }
        if let Some(dt) = self.started_before {
            qb.push(" AND started_at <= ").push_bind(dt);
        }
    }

    pub async fn runs_matching(
        &self,
        limit: i64,
        offset: i64,
        conn: &mut PgConnection,
    ) -> Result<Vec<TestRun>> {
        let mut qb = QueryBuilder::new("SELECT * FROM test_run");
        self.push_where_clause(&mut qb);

        qb.push(" ORDER BY started_at DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        Ok(qb.build_query_as::<TestRun>().fetch_all(conn).await?)
    }

    pub async fn n_matching(&self, conn: &mut PgConnection) -> Result<i64> {
        let mut qb = QueryBuilder::new("SELECT COUNT(*) FROM test_run");
        self.push_where_clause(&mut qb);

        Ok(qb.build_query_scalar().fetch_one(conn).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{conn, db::Queryable};
    use chrono::Duration;
    use uuid::Uuid;

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_filters_by_name() -> Result<()> {
        let c = conn!();
        let name = unique("match-me");
        TestRun::init_unknown_initiator(&name, None, c).await?;
        TestRun::init_unknown_initiator(&unique("not-this-one"), None, c).await?;

        let filter = TestRunFilter {
            name: Some(name.clone()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].name(), name);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_filters_by_initiated_by() -> Result<()> {
        let c = conn!();
        let alice = unique("alice");
        let bob = unique("bob");
        TestRun::init("a", None, Some(&alice), c).await?;
        TestRun::init("b", None, Some(&bob), c).await?;

        let filter = TestRunFilter {
            initiated_by: Some(alice.clone()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].initiated_by(), Some(alice.as_str()));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_filters_by_time_range() -> Result<()> {
        let c = conn!();
        let initiated_by = unique("time-range");
        let old = TestRun::init("old", None, Some(&initiated_by), c).await?;
        let recent = TestRun::init("recent", None, Some(&initiated_by), c).await?;

        sqlx::query("UPDATE test_run SET started_at = NOW() - INTERVAL '2 days' WHERE id = $1")
            .bind(old.id())
            .execute(&mut *c)
            .await?;

        let filter = TestRunFilter {
            initiated_by: Some(initiated_by),
            started_after: Some(Utc::now() - Duration::days(1)),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].uuid(), recent.uuid());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_respects_pagination() -> Result<()> {
        let c = conn!();
        let initiated_by = unique("pagination");
        for name in ["a", "b", "c"] {
            TestRun::init(name, None, Some(&initiated_by), c).await?;
        }

        let filter = TestRunFilter {
            initiated_by: Some(initiated_by),
            ..Default::default()
        };

        let page = filter.runs_matching(1, 1, c).await?;

        assert_eq!(page.len(), 1, "{page:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_orders_newest_first() -> Result<()> {
        let c = conn!();
        let initiated_by = unique("ordering");
        let first = TestRun::init("first", None, Some(&initiated_by), c).await?;
        TestRun::init("second", None, Some(&initiated_by), c).await?;

        sqlx::query("UPDATE test_run SET started_at = NOW() - INTERVAL '1 hour' WHERE id = $1")
            .bind(first.id())
            .execute(&mut *c)
            .await?;

        let filter = TestRunFilter {
            initiated_by: Some(initiated_by),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs[0].name(), "second", "{runs:?}");
        assert_eq!(runs[1].name(), "first", "{runs:?}");

        Ok(())
    }
}
