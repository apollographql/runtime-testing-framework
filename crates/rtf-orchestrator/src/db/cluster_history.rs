//! Aggregates over recent executions for each workload cluster.
use crate::db::{ClusterId, Result};
use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use rtf_orchestrator_shared::cluster_summary::HourlyCount;
use sqlx::PgConnection;
use std::collections::HashMap;

/// Counts of executions started per hour for each of the given clusters, oldest first.
pub async fn hourly_execution_counts(
    clusters: &[ClusterId],
    now: DateTime<Utc>,
    conn: &mut PgConnection,
) -> Result<HashMap<ClusterId, Vec<HourlyCount>>> {
    let current_hour = now
        .duration_trunc(TimeDelta::hours(1))
        .expect("round to hour is valid");
    let hours: Vec<_> = (0..24)
        .rev()
        .map(|i| current_hour - TimeDelta::hours(i))
        .collect();

    let rows: Vec<HourRow> = sqlx::query_as(
        r#"
        SELECT
          tr.workload_cluster                          AS cluster,
          date_trunc('hour', te.started_at, 'UTC')     AS hour,
          COUNT(*)                                     AS count
        FROM
          test_execution te
        JOIN test_run tr
          ON tr.id = te.test_run_id
        WHERE
          te.started_at >= $1
        GROUP BY
          cluster, hour;
    "#,
    )
    .bind(hours[0])
    .fetch_all(conn)
    .await?;

    let mut raw: HashMap<(String, DateTime<Utc>), u64> = rows
        .into_iter()
        .map(|row| ((row.cluster, row.hour), row.count as u64))
        .collect();

    Ok(clusters
        .iter()
        .map(|cid| {
            let counts = hours
                .iter()
                .map(|&hour| HourlyCount {
                    hour,
                    count: raw.remove(&(cid.to_string(), hour)).unwrap_or(0),
                })
                .collect();

            (cid.clone(), counts)
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct HourRow {
    cluster: String,
    hour: DateTime<Utc>,
    count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conn,
        db::{PoolId, Queryable, TestRun},
    };
    use uuid::Uuid;

    fn unique_cluster() -> ClusterId {
        ClusterId::new(format!("cluster-{}", Uuid::new_v4()))
    }

    async fn execution_started(
        cluster: &ClusterId,
        hours_ago: i32,
        conn: &mut PgConnection,
    ) -> Result<()> {
        let pool = PoolId::new(cluster.as_str());
        let tr =
            TestRun::init_unknown_initiator(&Uuid::new_v4().to_string(), None, &pool, conn).await?;
        let ex = tr
            .init_execution(&Uuid::new_v4().to_string(), 0, conn)
            .await?;

        sqlx::query(
            "UPDATE test_execution SET started_at = NOW() - make_interval(hours => $1) WHERE id = $2",
        )
        .bind(hours_ago)
        .bind(ex.id())
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    fn counts_for(counts: &HashMap<ClusterId, Vec<HourlyCount>>, cid: &ClusterId) -> Vec<u64> {
        counts[cid].iter().map(|c| c.count).collect()
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn hourly_execution_counts_buckets_executions_per_cluster() -> Result<()> {
        let c = conn!();
        let (a, b) = (unique_cluster(), unique_cluster());

        execution_started(&a, 0, c).await?;
        execution_started(&a, 0, c).await?;
        execution_started(&a, 2, c).await?;
        execution_started(&b, 1, c).await?;

        let counts = hourly_execution_counts(&[a.clone(), b.clone()], Utc::now(), c).await?;

        let mut expected_a = vec![0; 24];
        expected_a[23] = 2;
        expected_a[21] = 1;
        let mut expected_b = vec![0; 24];
        expected_b[22] = 1;

        assert_eq!(counts_for(&counts, &a), expected_a);
        assert_eq!(counts_for(&counts, &b), expected_b);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn hourly_execution_counts_ignores_executions_outside_the_window() -> Result<()> {
        let c = conn!();
        let cid = unique_cluster();

        execution_started(&cid, 30, c).await?;

        let counts = hourly_execution_counts(std::slice::from_ref(&cid), Utc::now(), c).await?;

        assert_eq!(counts_for(&counts, &cid), vec![0; 24]);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn hourly_execution_counts_zero_fills_clusters_with_no_executions() -> Result<()> {
        let c = conn!();
        let cid = unique_cluster();

        let counts = hourly_execution_counts(std::slice::from_ref(&cid), Utc::now(), c).await?;
        let hours: Vec<_> = counts[&cid].iter().map(|c| c.hour).collect();

        assert_eq!(counts_for(&counts, &cid), vec![0; 24]);
        assert!(hours.is_sorted(), "hours should be oldest first: {hours:?}");

        Ok(())
    }
}
