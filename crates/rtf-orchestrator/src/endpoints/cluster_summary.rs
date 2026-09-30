//! Summary of the configured workload clusters along with their recent execution counts.
use crate::{
    Result,
    config::{ClusterExecutionConfig, Config, PerUserExecutionConfig, WorkloadClusters},
    conn,
    db::{ClusterId, cluster_history::hourly_execution_counts},
};
use axum::Json;
use cached::proc_macro::cached;
use chrono::Utc;
use rtf_orchestrator_shared::cluster_summary::{
    ClusterExecutionSummary, ClusterSummaryResponse, HourlyCount, PerUserExecutionSummary,
    WorkloadClusterSummary, WorkloadPoolSummary,
};
use std::collections::HashMap;

pub async fn handler() -> Result<Json<ClusterSummaryResponse>> {
    let clusters = &Config::get().workload_clusters;
    let hourly = cached_hourly_execution_counts(clusters.available_clusters()).await?;

    Ok(Json(cluster_summary(clusters, hourly)))
}

#[cached(ttl_secs = 600)]
async fn cached_hourly_execution_counts(
    clusters: Vec<ClusterId>,
) -> Result<HashMap<ClusterId, Vec<HourlyCount>>> {
    Ok(hourly_execution_counts(&clusters, Utc::now(), conn!()).await?)
}

fn cluster_summary(
    clusters: &WorkloadClusters,
    mut hourly: HashMap<ClusterId, Vec<HourlyCount>>,
) -> ClusterSummaryResponse {
    let pools = clusters
        .cluster_pools
        .iter()
        .map(|(name, pool)| WorkloadPoolSummary {
            name: name.to_string(),
            clusters: pool
                .available_clusters
                .iter()
                .filter_map(|cluster| {
                    clusters
                        .available_clusters
                        .iter()
                        .find(|c| c.name == *cluster)
                })
                .map(|cfg| WorkloadClusterSummary {
                    name: cfg.name.clone(),
                    execution: execution_summary(&cfg.execution, &pool.per_user),
                    hourly_executions: hourly
                        .remove(&ClusterId::new(&cfg.name))
                        .unwrap_or_default(),
                })
                .collect(),
        })
        .collect();

    ClusterSummaryResponse {
        max_queued_executions: clusters.max_queued_executions,
        pools,
    }
}

fn execution_summary(
    cfg: &ClusterExecutionConfig,
    per_user: &PerUserExecutionConfig,
) -> ClusterExecutionSummary {
    ClusterExecutionSummary {
        max_concurrent: cfg.max_concurrent,
        failed_execution_ttl_secs: cfg.failed_execution_ttl_secs,
        retry_window_secs: cfg.retry_window_secs,
        poll_interval_secs: cfg.poll_interval_secs,
        exclusive_nodes: cfg.exclusive_nodes,
        scenario_node_selector: cfg.scenario_node_selector.clone(),
        namespace_cleanup_timeout_secs: cfg.namespace_cleanup_timeout_secs,
        per_user: PerUserExecutionSummary {
            max_concurrent_runs: per_user.max_concurrent_runs,
            max_queued_runs: per_user.max_queued_runs,
            max_queued_executions: per_user.max_queued_executions,
            max_runs_per_hour: per_user.max_runs_per_hour,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClusterPools, NamedPoolConfig, PoolConfig};
    use simple_test_case::test_case;

    fn pool_layout(resp: &ClusterSummaryResponse) -> Vec<(&str, Vec<&str>)> {
        resp.pools
            .iter()
            .map(|p| {
                let names = p.clusters.iter().map(|c| c.name.as_str()).collect();
                (p.name.as_str(), names)
            })
            .collect()
    }

    fn pool(clusters: &[&str]) -> PoolConfig {
        PoolConfig {
            dedicated: false,
            available_clusters: clusters.iter().map(|c| c.to_string()).collect(),
            per_user: Default::default(),
        }
    }

    #[test_case(pool(&["a"]), vec![], &[("default", &["a"])]; "default only")]
    #[test_case(
        pool(&["a"]),
        vec![NamedPoolConfig { name: "perf".into(), config: pool(&["b", "c"]) }],
        &[("default", &["a"]), ("perf", &["b", "c"])];
        "default and an additional pool with several clusters"
    )]
    #[test]
    fn cluster_summary_follows_the_configured_pools(
        default: PoolConfig,
        additional: Vec<NamedPoolConfig>,
        expected: &[(&str, &[&str])],
    ) {
        let mut clusters =
            WorkloadClusters::for_test_with_available_clusters(10, "a", &["a", "b", "c"]);
        clusters.cluster_pools = ClusterPools {
            default,
            additional,
        };

        let resp = cluster_summary(&clusters, HashMap::new());
        let expected: Vec<_> = expected.iter().map(|(p, cs)| (*p, cs.to_vec())).collect();

        assert_eq!(pool_layout(&resp), expected);
    }
}
