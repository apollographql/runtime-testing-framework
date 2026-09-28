use chrono::{DateTime, Utc};
use humantime::format_duration;
use rtf_orchestrator_shared::{
    cluster_summary::{ClusterExecutionSummary, ClusterSummaryResponse},
    event_queue::EventQueueSnapshot,
};
use std::time::Duration;

const GAUGE_WARN_PERCENT: usize = 75;

#[derive(Debug)]
pub struct DashboardView {
    pub queued: usize,
    pub max_queued: usize,
    pub running: usize,
    pub gauge_percent: usize,
    pub gauge_class: &'static str,
    pub pools: Vec<PoolView>,
    pub cluster_roles_url: String,
    hours: Vec<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct PoolView {
    pub name: String,
    pub clusters: Vec<ClusterView>,
}

#[derive(Debug)]
pub struct ClusterView {
    pub name: String,
    pub running: usize,
    pub max_concurrent: usize,
    pub utilisation_percent: usize,
    pub pending_provisions: usize,
    pub pending_other: usize,
    pub config: Vec<(&'static str, String)>,
    hourly_counts: Vec<u64>,
}

#[derive(Debug, serde::Serialize)]
pub struct DashboardJsData<'a> {
    hours: &'a [DateTime<Utc>],
    clusters: Vec<ClusterJsData<'a>>,
}

#[derive(Debug, serde::Serialize)]
struct ClusterJsData<'a> {
    name: &'a str,
    counts: &'a [u64],
}

impl DashboardView {
    pub fn new(
        mut snapshot: EventQueueSnapshot,
        summary: ClusterSummaryResponse,
        cluster_roles_url: String,
    ) -> Self {
        let hours = summary
            .pools
            .iter()
            .flat_map(|p| p.clusters.first())
            .map(|c| c.hourly_executions.iter().map(|h| h.hour).collect())
            .next()
            .unwrap_or_default();

        let pools = summary
            .pools
            .into_iter()
            .map(|pool| PoolView {
                name: pool.name,
                clusters: pool
                    .clusters
                    .into_iter()
                    .map(|cluster| {
                        let queue = snapshot.clusters.remove(&cluster.name).unwrap_or_default();
                        let running = queue.running_executions.len();
                        let max_concurrent = cluster.execution.max_concurrent;

                        ClusterView {
                            running,
                            max_concurrent,
                            utilisation_percent: percent(running, max_concurrent),
                            pending_provisions: queue.pending_provisions.len(),
                            pending_other: queue.pending_non_provisions.len(),
                            config: config_rows(&cluster.execution),
                            hourly_counts: cluster
                                .hourly_executions
                                .iter()
                                .map(|h| h.count)
                                .collect(),
                            name: cluster.name,
                        }
                    })
                    .collect(),
            })
            .collect();

        let queued = snapshot.summary.queued;
        let max_queued = summary.max_queued_executions;
        let gauge_percent = percent(queued, max_queued);
        let gauge_class = match gauge_percent {
            100.. => "gauge-full",
            GAUGE_WARN_PERCENT.. => "gauge-warn",
            _ => "gauge-ok",
        };

        Self {
            queued,
            max_queued,
            running: snapshot.summary.running,
            gauge_percent,
            gauge_class,
            pools,
            cluster_roles_url,
            hours,
        }
    }

    pub fn js_data(&self) -> DashboardJsData<'_> {
        DashboardJsData {
            hours: &self.hours,
            clusters: self
                .pools
                .iter()
                .flat_map(|p| p.clusters.iter())
                .map(|c| ClusterJsData {
                    name: &c.name,
                    counts: &c.hourly_counts,
                })
                .collect(),
        }
    }
}

/// Clamped to 100 so that over-subscription still renders as a full gauge.
fn percent(n: usize, max: usize) -> usize {
    match max {
        0 if n == 0 => 0,
        0 => 100,
        _ => (n * 100 / max).min(100),
    }
}

fn config_rows(cfg: &ClusterExecutionSummary) -> Vec<(&'static str, String)> {
    let secs = |s: u64| format_duration(Duration::from_secs(s)).to_string();
    let node_selector = if cfg.scenario_node_selector.is_empty() {
        "None".to_owned()
    } else {
        cfg.scenario_node_selector
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    vec![
        ("Max concurrent executions", cfg.max_concurrent.to_string()),
        (
            "Exclusive nodes",
            if cfg.exclusive_nodes { "Yes" } else { "No" }.to_owned(),
        ),
        ("Scenario node selector", node_selector),
        ("Failed execution TTL", secs(cfg.failed_execution_ttl_secs)),
        ("Retry window", secs(cfg.retry_window_secs)),
        ("Poll interval", secs(cfg.poll_interval_secs)),
        (
            "Namespace cleanup timeout",
            secs(cfg.namespace_cleanup_timeout_secs),
        ),
        (
            "Max concurrent runs per user",
            cfg.per_user.max_concurrent_runs.to_string(),
        ),
        (
            "Max queued runs per user",
            cfg.per_user.max_queued_runs.to_string(),
        ),
        (
            "Max queued executions per user",
            cfg.per_user.max_queued_executions.to_string(),
        ),
        (
            "Max runs per hour per user",
            cfg.per_user.max_runs_per_hour.to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::mocks::{sample_cluster_summary, sample_event_queue_snapshot};
    use simple_test_case::test_case;

    fn dashboard() -> DashboardView {
        DashboardView::new(
            sample_event_queue_snapshot(),
            sample_cluster_summary(),
            String::new(),
        )
    }

    #[test_case(0, 0, 0; "nothing allowed and nothing queued")]
    #[test_case(3, 0, 100; "nothing allowed but something queued")]
    #[test_case(5, 20, 25; "partial")]
    #[test_case(30, 20, 100; "over capacity is clamped")]
    #[test]
    fn percent_cases(n: usize, max: usize, expected: usize) {
        assert_eq!(percent(n, max), expected);
    }

    #[test_case(4, "gauge-ok"; "below warning threshold")]
    #[test_case(15, "gauge-warn"; "at warning threshold")]
    #[test_case(20, "gauge-full"; "at capacity")]
    #[test]
    fn gauge_class_reflects_queue_depth(queued: usize, expected: &str) {
        let mut snapshot = sample_event_queue_snapshot();
        snapshot.summary.queued = queued;

        let d = DashboardView::new(snapshot, sample_cluster_summary(), String::new());

        assert_eq!(d.gauge_class, expected);
    }

    #[test]
    fn queue_state_is_joined_onto_each_cluster() {
        let d = dashboard();
        let clusters: Vec<_> = d
            .pools
            .iter()
            .flat_map(|p| p.clusters.iter().map(move |c| (p.name.as_str(), c)))
            .map(|(pool, c)| {
                (
                    pool,
                    c.name.as_str(),
                    c.running,
                    c.pending_provisions,
                    c.pending_other,
                )
            })
            .collect();

        assert_eq!(
            clusters,
            vec![
                ("default", "alpha", 2, 2, 1),
                ("dedicated", "beta", 1, 0, 0)
            ]
        );
    }

    #[test]
    fn clusters_missing_from_the_snapshot_have_an_empty_queue() {
        let d = DashboardView::new(
            EventQueueSnapshot::default(),
            sample_cluster_summary(),
            String::new(),
        );

        assert!(
            d.pools
                .iter()
                .flat_map(|p| p.clusters.iter())
                .all(|c| c.running == 0 && c.pending_provisions == 0 && c.pending_other == 0)
        );
    }
}
