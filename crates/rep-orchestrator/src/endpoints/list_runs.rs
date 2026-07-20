//! List historic test runs matching a set of optional query filters.
use crate::{Result, conn, db::TestRunFilter};
use axum::{Json, extract::Query};
use chrono::{DateTime, Utc};
use rep_orchestrator_shared::summary::TestRunListResponse;
use serde::Deserialize;

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Deserialize)]
pub struct Params {
    name: Option<String>,
    initiated_by: Option<String>,
    started_after: Option<DateTime<Utc>>,
    started_before: Option<DateTime<Utc>>,
    limit: Option<i64>,
    offset: Option<i64>,
}

pub async fn handler(Query(params): Query<Params>) -> Result<Json<TestRunListResponse>> {
    let conn = conn!();

    let filters = TestRunFilter {
        name: params.name,
        initiated_by: params.initiated_by,
        started_after: params.started_after,
        started_before: params.started_before,
    };

    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = params.offset.unwrap_or(0).max(0);

    let runs = filters.runs_matching(limit, offset, conn).await?;
    let total = filters.n_matching(conn).await?;

    let mut summaries = Vec::with_capacity(runs.len());

    for run in runs.into_iter() {
        summaries.push(run.try_into_summary(conn).await?);
    }

    Ok(Json(TestRunListResponse {
        runs: summaries,
        total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{Queryable, TestRun},
        test_helpers::TestServerState,
    };
    use chrono::Duration;
    use reqwest::StatusCode;
    use uuid::Uuid;

    // Tests run against a shared, long-lived dev database rather than a fresh one per test, so
    // every identifier used as a query filter must be unique to this test invocation. Otherwise
    // rows left behind by other tests (past or concurrently running) would leak into totals and
    // break these assertions. `unique` gives each test a value no other run could plausibly share.
    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_matching_runs_with_no_other_filters() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let initiated_by = unique("returns-matching-runs");
        TestRun::init("a", None, Some(&initiated_by), conn).await?;
        TestRun::init("b", None, Some(&initiated_by), conn).await?;

        // Scope the otherwise-unfiltered request with a filter unique to this test, since the
        // table also holds rows from every other test that has run against this database.
        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("initiated_by", &initiated_by)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.runs.len(), 2, "{body:?}");
        assert_eq!(body.total, 2, "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_filters_by_name() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let name = unique("match-me");
        TestRun::init_unknown_initiator(&name, None, conn).await?;
        TestRun::init_unknown_initiator(&unique("not-this-one"), None, conn).await?;

        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("name", &name)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.total, 1, "{body:?}");
        assert_eq!(body.runs[0].name, name);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_filters_by_initiated_by() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let alice = unique("alice");
        let bob = unique("bob");
        TestRun::init("a", None, Some(&alice), conn).await?;
        TestRun::init("b", None, Some(&bob), conn).await?;

        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("initiated_by", &alice)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.total, 1, "{body:?}");
        assert_eq!(body.runs[0].initiated_by, alice);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_respects_limit_and_offset() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let initiated_by = unique("respects-limit-and-offset");
        for name in ["a", "b", "c"] {
            TestRun::init(name, None, Some(&initiated_by), conn).await?;
        }

        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("initiated_by", &initiated_by)
            .add_query_param("limit", 1)
            .add_query_param("offset", 1)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.runs.len(), 1, "{body:?}");
        assert_eq!(body.total, 3, "total should ignore pagination: {body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_orders_newest_first() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let initiated_by = unique("orders-newest-first");
        let first = TestRun::init("first", None, Some(&initiated_by), conn).await?;
        TestRun::init("second", None, Some(&initiated_by), conn).await?;

        // Force a deterministic ordering regardless of how fast the two inserts above ran.
        sqlx::query("UPDATE test_run SET started_at = NOW() - INTERVAL '1 hour' WHERE id = $1")
            .bind(first.id())
            .execute(&mut *conn)
            .await?;

        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("initiated_by", &initiated_by)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.runs[0].name, "second", "{body:?}");
        assert_eq!(body.runs[1].name, "first", "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_filters_by_started_after() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let initiated_by = unique("started-after");
        let old = TestRun::init("old", None, Some(&initiated_by), conn).await?;
        TestRun::init("recent", None, Some(&initiated_by), conn).await?;

        sqlx::query("UPDATE test_run SET started_at = NOW() - INTERVAL '2 days' WHERE id = $1")
            .bind(old.id())
            .execute(&mut *conn)
            .await?;

        let cutoff = Utc::now() - Duration::days(1);
        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("initiated_by", &initiated_by)
            .add_query_param("started_after", cutoff.to_rfc3339())
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.total, 1, "{body:?}");
        assert_eq!(body.runs[0].name, "recent", "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_filters_by_started_before() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let initiated_by = unique("started-before");
        let old = TestRun::init("old", None, Some(&initiated_by), conn).await?;
        TestRun::init("recent", None, Some(&initiated_by), conn).await?;

        sqlx::query("UPDATE test_run SET started_at = NOW() - INTERVAL '2 days' WHERE id = $1")
            .bind(old.id())
            .execute(&mut *conn)
            .await?;

        let cutoff = Utc::now() - Duration::days(1);
        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("initiated_by", &initiated_by)
            .add_query_param("started_before", cutoff.to_rfc3339())
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.total, 1, "{body:?}");
        assert_eq!(body.runs[0].name, "old", "{body:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_does_not_populate_executions() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let conn = conn!();
        let name = unique("test");
        let tr = TestRun::init_unknown_initiator(&name, None, conn).await?;
        tr.init_execution("exec", 0, conn).await?;

        let resp = tss
            .test_server
            .get("/test-run")
            .add_query_param("name", &name)
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK);

        let body: TestRunListResponse = resp.json();

        assert_eq!(body.runs.len(), 1, "{body:?}");
        assert!(body.runs[0].executions.is_empty(), "{body:?}");

        Ok(())
    }
}
