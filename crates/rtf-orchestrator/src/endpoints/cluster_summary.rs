//! Summary of the configured workload clusters along with their recent execution counts.
use crate::{
    Result,
    config::{
        ClusterExecutionConfig, Config, DEFAULT_POOL, PerUserExecutionConfig, WorkloadClusters,
    },
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

// For now we are grouping into cluster pools here with a default pool for alpha and a second
// "dedicate" pool for everything else. The changes proposed in RR-1175 will make this the native
// representation in server config so this is a temporary shim to ensure that we don't need to
// rework the UI side of things once we move over to actually using pools.
fn cluster_summary(
    clusters: &WorkloadClusters,
    mut hourly: HashMap<ClusterId, Vec<HourlyCount>>,
) -> ClusterSummaryResponse {
    let mut default_pool = WorkloadPoolSummary {
        name: DEFAULT_POOL.to_string(),
        clusters: Vec::new(),
    };
    let mut perf_pool = WorkloadPoolSummary {
        name: "perf".to_string(),
        clusters: Vec::new(),
    };

    for cfg in clusters.available_clusters.iter() {
        let summary = WorkloadClusterSummary {
            name: cfg.name.clone(),
            execution: execution_summary(
                &cfg.execution,
                clusters
                    .cluster_pools
                    .pool_config_for_cluster(&cfg.name)
                    .map(|pool| pool.per_user.clone())
                    .unwrap_or_default(),
            ),
            hourly_executions: hourly
                .remove(&ClusterId::new(&cfg.name))
                .unwrap_or_default(),
        };

        if clusters
            .cluster_pools
            .default
            .available_clusters
            .contains(&cfg.name)
        {
            default_pool.clusters.push(summary);
        } else {
            perf_pool.clusters.push(summary);
        }
    }

    ClusterSummaryResponse {
        max_queued_executions: clusters.max_queued_executions,
        pools: [default_pool, perf_pool]
            .into_iter()
            .filter(|pool| !pool.clusters.is_empty())
            .collect(),
    }
}

fn execution_summary(
    cfg: &ClusterExecutionConfig,
    per_user: PerUserExecutionConfig,
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

    #[test_case("alpha", &["alpha"], &[("default", &["alpha"])]; "default only")]
    #[test_case(
        "alpha",
        &["alpha", "beta", "other"],
        &[("default", &["alpha"]), ("perf", &["beta", "other"])];
        "default and perf"
    )]
    #[test]
    fn cluster_summary_groups_clusters_into_pools(
        default_cluster: &str,
        names: &[&str],
        expected: &[(&str, &[&str])],
    ) {
        let clusters =
            WorkloadClusters::for_test_with_available_clusters(10, default_cluster, names);

        let resp = cluster_summary(&clusters, HashMap::new());
        let expected: Vec<_> = expected.iter().map(|(p, cs)| (*p, cs.to_vec())).collect();

        assert_eq!(pool_layout(&resp), expected);
    }
}
