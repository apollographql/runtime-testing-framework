//! Summary of the configured workload clusters along with their recent execution counts.
use crate::{
    Result,
    config::{
        ClusterExecutionConfig, Config, PerUserExecutionConfig, WorkloadClusterConfig,
        WorkloadClusters,
    },
    conn,
    db::{ClusterId, cluster_history::hourly_execution_counts},
    k8s::{self, WorkloadClient},
};
use axum::Json;
use cached::proc_macro::cached;
use chrono::Utc;
use futures::future::join_all;
use rtf_orchestrator_shared::cluster_summary::{
    ClusterExecutionSummary, ClusterSummaryResponse, HourlyCount, NodesSummary,
    PerUserExecutionSummary, WorkloadClusterSummary, WorkloadPoolSummary,
};
use std::collections::HashMap;
use tracing::warn;

pub async fn handler() -> Result<Json<ClusterSummaryResponse>> {
    let clusters = &Config::get().workload_clusters;
    let hourly = cached_hourly_execution_counts(clusters.available_clusters()).await?;
    let nodes = cached_node_summaries(clusters.available_clusters()).await;

    Ok(Json(cluster_summary(clusters, hourly, nodes)))
}

#[cached(ttl_secs = 600)]
async fn cached_hourly_execution_counts(
    clusters: Vec<ClusterId>,
) -> Result<HashMap<ClusterId, Vec<HourlyCount>>> {
    Ok(hourly_execution_counts(&clusters, Utc::now(), conn!()).await?)
}

#[cached(ttl_secs = 600)]
async fn cached_node_summaries(clusters: Vec<ClusterId>) -> HashMap<ClusterId, NodesSummary> {
    let per_cluster = Config::get().workload_clusters.per_cluster_config();

    let fetches = clusters.iter().filter_map(|id| {
        per_cluster.get(id).map(|cfg| async move {
            cluster_node_summary(cfg)
                .await
                .map(|summary| (id.clone(), summary))
        })
    });

    join_all(fetches).await.into_iter().flatten().collect()
}

/// `None` when the cluster's nodes could not be fetched (e.g. it is unreachable) - a single
/// broken cluster should not take down the whole summary.
async fn cluster_node_summary(cfg: &WorkloadClusterConfig) -> Option<NodesSummary> {
    let mut clients =
        k8s::ClusterClients::try_new_workload(&cfg.kubeconfig_path(), &cfg.workload_context)
            .await
            .inspect_err(|e| warn!(cluster = %cfg.name, %e, "failed to build workload client"))
            .ok()?;

    clients
        .nodes_summary()
        .await
        .inspect_err(|e| warn!(cluster = %cfg.name, %e, "failed to fetch node summary"))
        .ok()
}

fn cluster_summary(
    clusters: &WorkloadClusters,
    mut hourly: HashMap<ClusterId, Vec<HourlyCount>>,
    mut nodes: HashMap<ClusterId, NodesSummary>,
) -> ClusterSummaryResponse {
    let pools = clusters
        .cluster_pools
        .iter()
        .map(|(name, pool)| WorkloadPoolSummary {
            name: name.to_string(),
            supports_dedicated: pool.supports_dedicated,
            per_user: per_user_summary(&pool.per_user_limits),
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
                    execution: execution_summary(&cfg.execution),
                    hourly_executions: hourly
                        .remove(&ClusterId::new(&cfg.name))
                        .unwrap_or_default(),
                    nodes: nodes.remove(&ClusterId::new(&cfg.name)),
                })
                .collect(),
        })
        .collect();

    ClusterSummaryResponse {
        max_queued_executions: clusters.max_queued_executions,
        pools,
    }
}

fn per_user_summary(per_user: &PerUserExecutionConfig) -> PerUserExecutionSummary {
    PerUserExecutionSummary {
        max_concurrent_runs: per_user.max_concurrent_runs,
        max_queued_runs: per_user.max_queued_runs,
        max_queued_executions: per_user.max_queued_executions,
        max_runs_per_hour: per_user.max_runs_per_hour,
    }
}

fn execution_summary(cfg: &ClusterExecutionConfig) -> ClusterExecutionSummary {
    ClusterExecutionSummary {
        max_concurrent: cfg.max_concurrent,
        failed_execution_ttl_secs: cfg.failed_execution_ttl_secs,
        retry_window_secs: cfg.retry_window_secs,
        poll_interval_secs: cfg.poll_interval_secs,
        exclusive_nodes: cfg.exclusive_nodes,
        scenario_node_selector: cfg.scenario_node_selector.clone(),
        namespace_cleanup_timeout_secs: cfg.namespace_cleanup_timeout_secs,
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
            supports_dedicated: false,
            available_clusters: clusters.iter().map(|c| c.to_string()).collect(),
            per_user_limits: Default::default(),
            workload_config: Default::default(),
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

        let resp = cluster_summary(&clusters, HashMap::new(), HashMap::new());
        let expected: Vec<_> = expected.iter().map(|(p, cs)| (*p, cs.to_vec())).collect();

        assert_eq!(pool_layout(&resp), expected);
    }

    #[test]
    fn cluster_summary_reports_dedicated_support_and_user_limits_per_pool() {
        let mut dedicated = pool(&["a"]);
        dedicated.supports_dedicated = true;
        dedicated.per_user_limits.max_queued_runs = 7;
        let mut perf = pool(&["b"]);
        perf.per_user_limits.max_queued_runs = 3;
        let mut clusters = WorkloadClusters::for_test_with_available_clusters(10, "a", &["a", "b"]);
        clusters.cluster_pools = ClusterPools {
            default: dedicated,
            additional: vec![NamedPoolConfig {
                name: "perf".into(),
                config: perf,
            }],
        };

        let resp = cluster_summary(&clusters, HashMap::new(), HashMap::new());
        let pools: Vec<_> = resp
            .pools
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.supports_dedicated,
                    p.per_user.max_queued_runs,
                )
            })
            .collect();

        assert_eq!(pools, vec![("default", true, 7), ("perf", false, 3)]);
    }
}
