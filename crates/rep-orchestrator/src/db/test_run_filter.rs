use crate::db::{Result, TestRun, test_run::UNKNOWN_INITIATOR};
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
            // `NULL ILIKE anything` is `NULL` (never matches), so a run with no recorded
            // initiator would be unfindable no matter what's searched — even searching for
            // "unknown", which is exactly what such a run displays as. Coalescing to the same
            // placeholder the UI shows keeps search consistent with what's on screen.
            qb.push(" AND COALESCE(initiated_by, ")
                .push_bind(UNKNOWN_INITIATOR)
                .push(") ILIKE ")
                .push_bind(substring_pattern(user));
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

/// Builds an `ILIKE` pattern matching `term` as a case-insensitive substring anywhere in the
/// column, escaping `%`, `_`, and `\` in `term` first so they match literally rather than as `LIKE`
/// wildcards (Postgres's default `LIKE`/`ILIKE` escape character is `\`, so no `ESCAPE` clause is
/// needed on the query side).
fn substring_pattern(term: &str) -> String {
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
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

    #[test]
    fn substring_pattern_wraps_the_term_in_wildcards() {
        assert_eq!(substring_pattern("alice"), "%alice%");
    }

    #[test]
    fn substring_pattern_escapes_like_wildcards_in_the_term() {
        assert_eq!(substring_pattern("user_name"), "%user\\_name%");
        assert_eq!(substring_pattern("50%off"), "%50\\%off%");
        assert_eq!(substring_pattern("a\\b"), "%a\\\\b%");
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
    async fn runs_matching_filters_by_initiated_by_substring() -> Result<()> {
        let c = conn!();
        let alice = unique("alice");
        let bob = unique("bob");
        TestRun::init("a", None, Some(&alice), c).await?;
        TestRun::init("b", None, Some(&bob), c).await?;

        // A substring of `alice`'s unique value, not the full value.
        let needle = &alice[..alice.len() - 4];
        let filter = TestRunFilter {
            initiated_by: Some(needle.to_owned()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].initiated_by(), Some(alice.as_str()));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_filters_by_initiated_by_case_insensitively() -> Result<()> {
        let c = conn!();
        let alice = unique("alice");
        TestRun::init("a", None, Some(&alice), c).await?;

        let filter = TestRunFilter {
            initiated_by: Some(alice.to_uppercase()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].initiated_by(), Some(alice.as_str()));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_treats_underscore_in_initiated_by_literally() -> Result<()> {
        let c = conn!();
        // Without escaping, `_` is a LIKE single-character wildcard and `user_name` would also
        // match `user1name`.
        let user_name = unique("user_name");
        let user1name = format!(
            "user1name-{}",
            &user_name[user_name.rfind('-').unwrap() + 1..]
        );
        TestRun::init("a", None, Some(&user_name), c).await?;
        TestRun::init("b", None, Some(&user1name), c).await?;

        let filter = TestRunFilter {
            initiated_by: Some(user_name.clone()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].initiated_by(), Some(user_name.as_str()));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_initiated_by_treats_a_missing_initiator_as_unknown() -> Result<()> {
        let c = conn!();
        let name = unique("no-initiator");
        TestRun::init_unknown_initiator(&name, None, c).await?;

        // Scoped by `name` (unique to this test) since every other test's `init_unknown_initiator`
        // rows in this shared dev database would otherwise also match "unknown".
        let filter = TestRunFilter {
            name: Some(name.clone()),
            initiated_by: Some("unknown".to_owned()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_initiated_by_matches_a_substring_of_unknown_case_insensitively()
    -> Result<()> {
        let c = conn!();
        let name = unique("no-initiator-substring");
        TestRun::init_unknown_initiator(&name, None, c).await?;

        let filter = TestRunFilter {
            name: Some(name.clone()),
            initiated_by: Some("KNOW".to_owned()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 1, "{runs:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_matching_initiated_by_does_not_match_unrelated_terms_for_a_missing_initiator()
    -> Result<()> {
        let c = conn!();
        let name = unique("no-initiator-unrelated");
        TestRun::init_unknown_initiator(&name, None, c).await?;

        let filter = TestRunFilter {
            name: Some(name.clone()),
            initiated_by: Some("alice".to_owned()),
            ..Default::default()
        };
        let runs = filter.runs_matching(10, 0, c).await?;

        assert_eq!(runs.len(), 0, "{runs:?}");

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
