//! Aggregates over a registered test plan's historic runs.
//!
//! These are deliberately targeted queries rather than reads of whole [TestRun][crate::db::TestRun]
//! and [TestExecution][crate::db::TestExecution] rows: a busy test plan accumulates far more history
//! than we would want to pull into memory to summarise.
use crate::db::{KnownTestPlan, Result, Status};
use chrono::NaiveDate;
use rtf_orchestrator_shared::test_plan_details::{
    DAY_FORMAT, DayCounts, DurationBin, DurationHistogram, HistoryWindow, TestPlanHistory,
};
use sqlx::PgConnection;
use std::collections::BTreeMap;

/// Number of bins in the execution duration histogram.
const DURATION_BINS: i64 = 10;

impl KnownTestPlan {
    pub async fn test_plan_history(
        &self,
        window: HistoryWindow,
        conn: &mut PgConnection,
    ) -> Result<TestPlanHistory> {
        let runs_by_day = self.runs_by_day(window, conn).await?;
        let execution_durations = self.execution_durations(window, conn).await?;

        Ok(TestPlanHistory {
            window,
            runs_by_day,
            execution_durations,
        })
    }

    async fn runs_by_day(
        &self,
        win: HistoryWindow,
        conn: &mut PgConnection,
    ) -> Result<BTreeMap<String, DayCounts>> {
        #[derive(sqlx::FromRow)]
        struct DayRow {
            day: NaiveDate,
            successful: i32,
            failed: i32,
            unrunnable: i32,
        }

        // The `JOIN LATERAL` here picks out each run's most recent status, with filtering on the
        // terminal statuses to omit out runs that are still in flight.
        let rows: Vec<DayRow> = sqlx::query_as(
            r#"
            SELECT
              (tr.started_at AT TIME ZONE 'UTC')::date         AS day,
              COUNT(*) FILTER (WHERE latest.status = $4)::int  AS successful,
              COUNT(*) FILTER (WHERE latest.status = $5)::int  AS failed,
              COUNT(*) FILTER (WHERE latest.status = $6)::int  AS unrunnable
            FROM
              test_run tr
            JOIN known_test_plan_run ktpr
              ON ktpr.test_run_id = tr.id
            JOIN known_test_plan ktp
              ON ktp.id = ktpr.known_test_plan_id
            JOIN LATERAL (
              SELECT
                status
              FROM
                test_run_status
              WHERE
                parent_id = tr.id
              ORDER BY
                updated_at DESC
              LIMIT 1
            ) latest ON TRUE
            WHERE
              ktp.uuid = $1
              AND tr.started_at >= $2
              AND tr.started_at <  $3
              AND latest.status IN ($4, $5, $6)
            GROUP BY
              day
            ORDER BY
              day;
        "#,
        )
        .bind(self.uuid())
        .bind(win.from)
        .bind(win.to)
        .bind(Status::Successful)
        .bind(Status::Failed)
        .bind(Status::Unrunnable)
        .fetch_all(conn)
        .await?;

        let mut counts: BTreeMap<String, DayCounts> = rows
            .into_iter()
            .map(|row| {
                (
                    row.day.format(DAY_FORMAT).to_string(),
                    DayCounts {
                        successful: row.successful as u64,
                        failed: row.failed as u64,
                        unrunnable: row.unrunnable as u64,
                    },
                )
            })
            .collect();

        // ensure that we have a set of counts for every day in the requested range
        for day in win.days() {
            counts.entry(day).or_default();
        }

        Ok(counts)
    }

    async fn execution_durations(
        &self,
        window: HistoryWindow,
        conn: &mut PgConnection,
    ) -> Result<DurationHistogram> {
        #[derive(sqlx::FromRow)]
        struct DurationRow {
            lo: i32,
            hi: i32,
            width: i32,
            bin: i32,
            count: i32,
        }

        let rows: Vec<DurationRow> = sqlx::query_as(
            r#"
            WITH durations AS (
              SELECT
                ROUND(EXTRACT(EPOCH FROM (te.completed_at - te.started_at)))::int AS secs
              FROM
                test_execution te
              JOIN test_run tr
                ON tr.id = te.test_run_id
              JOIN known_test_plan_run ktpr
                ON ktpr.test_run_id = tr.id
              JOIN known_test_plan ktp
                ON ktp.id = ktpr.known_test_plan_id
              WHERE
                ktp.uuid = $1
                AND tr.started_at >= $2
                AND tr.started_at <  $3
                AND te.completed_at IS NOT NULL
            ),
            bounds AS (
              SELECT
                MIN(secs) AS lo,
                MAX(secs) AS hi,
                GREATEST(1, CEIL((MAX(secs) - MIN(secs) + 1) / $4)::int) AS width
              FROM
                durations
            )
            SELECT
              b.lo AS lo,
              b.hi AS hi,
              b.width AS width,
              ((d.secs - b.lo) / b.width) AS bin,
              COUNT(*)::int AS count
            FROM
              durations d
            CROSS JOIN bounds b
            GROUP BY
              b.lo, b.hi, b.width, bin
            ORDER BY
              bin;
        "#,
        )
        .bind(self.uuid())
        .bind(window.from)
        .bind(window.to)
        .bind(DURATION_BINS)
        .fetch_all(conn)
        .await?;

        let first = match rows.first() {
            Some(first) => first,
            None => return Ok(DurationHistogram::default()),
        };

        let counts: BTreeMap<i32, u64> = rows.iter().map(|r| (r.bin, r.count as u64)).collect();
        let (lo, hi, width) = (first.lo, first.hi, first.width);
        let n_bins = (hi - lo + width) / width;

        let bins = (0..n_bins)
            .map(|i| {
                let lower = lo + i * width;

                DurationBin {
                    lower_secs: lower as u64,
                    upper_secs: (lower + width) as u64,
                    count: counts.get(&i).copied().unwrap_or(0),
                }
            })
            .collect();

        Ok(DurationHistogram {
            bins,
            min_secs: lo as u64,
            max_secs: hi as u64,
            total: counts.values().sum(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conn,
        db::{KnownTestPlan, KnownTestPlanRun, Queryable, StatusTracked, TestRun},
    };
    use chrono::{Duration, Utc};
    use rtf_orchestrator_shared::test_plan_details::TestPlanDetailsParams;
    use uuid::Uuid;

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn window(days_back: u32, days: u32) -> HistoryWindow {
        TestPlanDetailsParams {
            days_back,
            days,
            ..Default::default()
        }
        .history_window(Utc::now())
    }

    fn ten_day_window() -> HistoryWindow {
        window(10, 10)
    }

    async fn register_plan(conn: &mut PgConnection) -> Result<KnownTestPlan> {
        KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), conn).await
    }

    async fn run_started(
        plan: &KnownTestPlan,
        days_ago: i32,
        status: Status,
        conn: &mut PgConnection,
    ) -> Result<TestRun> {
        let tr = TestRun::init_unknown_initiator(&unique("run"), None, conn).await?;
        KnownTestPlanRun::link(plan.id(), tr.id(), None, conn).await?;

        tr.set_status(status, None, conn).await?;

        sqlx::query(
            "UPDATE test_run SET started_at = NOW() - make_interval(days => $1) WHERE id = $2",
        )
        .bind(days_ago)
        .bind(tr.id())
        .execute(&mut *conn)
        .await?;

        Ok(tr)
    }

    async fn execution_lasting(
        tr: &TestRun,
        secs: f64,
        index: usize,
        conn: &mut PgConnection,
    ) -> Result<()> {
        let ex = tr.init_execution(&unique("ex"), index, conn).await?;

        sqlx::query(
            "UPDATE test_execution
             SET started_at = NOW(), completed_at = NOW() + make_interval(secs => $1)
             WHERE id = $2",
        )
        .bind(secs)
        .bind(ex.id())
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    fn day_key(days_ago: i64) -> String {
        (Utc::now() - Duration::days(days_ago))
            .format(DAY_FORMAT)
            .to_string()
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_by_day_counts_each_final_status() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;

        run_started(&plan, 1, Status::Successful, c).await?;
        run_started(&plan, 1, Status::Successful, c).await?;
        run_started(&plan, 2, Status::Failed, c).await?;
        run_started(&plan, 3, Status::Unrunnable, c).await?;

        let counts = plan.runs_by_day(ten_day_window(), c).await?;

        assert_eq!(counts.get(&day_key(1)).map(|c| c.successful), Some(2));
        assert_eq!(counts.get(&day_key(2)).map(|c| c.failed), Some(1));
        assert_eq!(counts.get(&day_key(3)).map(|c| c.unrunnable), Some(1));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_by_day_zero_fills_days_with_no_runs() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;

        run_started(&plan, 1, Status::Successful, c).await?;

        let counts = plan.runs_by_day(ten_day_window(), c).await?;

        assert_eq!(counts.len(), 10, "should be every day in the window");
        assert_eq!(counts.get(&day_key(4)), Some(&DayCounts::default()));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_by_day_excludes_runs_outside_the_window() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;

        // The window covers days 3..6 ago, so both of these sit just outside it.
        run_started(&plan, 7, Status::Successful, c).await?;
        run_started(&plan, 2, Status::Successful, c).await?;

        let counts = plan.runs_by_day(window(6, 3), c).await?;

        assert!(
            counts.values().all(|c| c.total() == 0),
            "expected no runs counted, got {counts:?}"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn runs_by_day_omits_runs_that_have_not_finished() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;

        run_started(&plan, 1, Status::Resolving, c).await?;
        run_started(&plan, 1, Status::Running, c).await?;

        let counts = plan.runs_by_day(ten_day_window(), c).await?;

        assert_eq!(counts.get(&day_key(1)), Some(&DayCounts::default()));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn history_is_empty_for_a_plan_with_no_runs() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;
        let w = ten_day_window();

        let history = plan.test_plan_history(w, c).await?;

        assert_eq!(history.window, w);
        assert!(history.runs_by_day.values().all(|c| c.total() == 0));
        assert_eq!(history.execution_durations, DurationHistogram::default());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn execution_durations_bins_across_the_full_range() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;
        let tr = run_started(&plan, 1, Status::Successful, c).await?;

        // Range 0..=99 over 10 bins gives a width of 10, so these land in bins 0, 0, 5 and 9.
        for (i, secs) in [0.0, 4.0, 55.0, 99.0].into_iter().enumerate() {
            execution_lasting(&tr, secs, i, c).await?;
        }

        let histogram = plan.execution_durations(ten_day_window(), c).await?;

        assert_eq!(histogram.min_secs, 0);
        assert_eq!(histogram.max_secs, 99);
        assert_eq!(histogram.total, 4);
        assert_eq!(histogram.bins.len(), DURATION_BINS as usize);
        assert_eq!(histogram.bins[0].count, 2, "{:?}", histogram.bins);
        assert_eq!(histogram.bins[1].count, 0, "{:?}", histogram.bins);
        assert_eq!(histogram.bins[5].count, 1, "{:?}", histogram.bins);
        assert_eq!(histogram.bins[9].count, 1, "{:?}", histogram.bins);
        assert_eq!(histogram.bins[9].upper_secs, 100);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn execution_durations_collapse_to_one_bin_when_all_equal() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;
        let tr = run_started(&plan, 1, Status::Successful, c).await?;

        for i in 0..3 {
            execution_lasting(&tr, 7.0, i, c).await?;
        }

        let histogram = plan.execution_durations(ten_day_window(), c).await?;

        assert_eq!(histogram.min_secs, 7);
        assert_eq!(histogram.max_secs, 7);
        assert_eq!(histogram.total, 3);
        assert_eq!(
            histogram.bins,
            vec![DurationBin {
                lower_secs: 7,
                upper_secs: 8,
                count: 3
            }]
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn execution_durations_ignore_executions_that_have_not_completed() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;
        let tr = run_started(&plan, 1, Status::Successful, c).await?;

        tr.init_execution(&unique("ex"), 0, c).await?;

        let histogram = plan.execution_durations(ten_day_window(), c).await?;

        assert_eq!(histogram, DurationHistogram::default());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn execution_durations_round_to_whole_seconds() -> Result<()> {
        let c = conn!();
        let plan = register_plan(c).await?;
        let tr = run_started(&plan, 1, Status::Successful, c).await?;

        execution_lasting(&tr, 2.4, 0, c).await?;
        execution_lasting(&tr, 2.6, 1, c).await?;

        let histogram = plan.execution_durations(ten_day_window(), c).await?;

        assert_eq!(histogram.min_secs, 2, "2.4s rounds down");
        assert_eq!(histogram.max_secs, 3, "2.6s rounds up");

        Ok(())
    }
}
