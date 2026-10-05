use chrono::{DateTime, Utc};
use humantime::format_duration;
use rtf_orchestrator_shared::{
    cluster_summary::{
        ClusterExecutionSummary, ClusterSummaryResponse, NodesSummary, PerUserExecutionSummary,
    },
    event_queue::{ClusterClaimSummary, EventQueueSnapshot},
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::Duration,
};
use uuid::Uuid;

const QUEUE_WARN_PERCENT: usize = 75;
const PALETTE_SIZE: usize = 12;

#[derive(Debug)]
pub struct DashboardView {
    pub queued: usize,
    pub max_queued: usize,
    pub running: usize,
    pub queue_percent: usize,
    pub queue_fill_modifier: &'static str,
    pub pools: Vec<PoolView>,
    pub cluster_roles_url: String,
    hours: Vec<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct PoolView {
    pub name: String,
    pub supports_dedicated: bool,
    pub queued_executions: usize,
    pub queued_runs: usize,
    pub queued_dedicated: usize,
    pub limits: Vec<(&'static str, String)>,
    pub clusters: Vec<ClusterView>,
}

#[derive(Debug)]
pub struct ClusterView {
    pub name: String,
    pub running: usize,
    pub max_concurrent: usize,
    pub utilisation_percent: usize,
    pub pending_other: usize,
    pub claim: Option<ClaimView>,
    pub config: Vec<(&'static str, String)>,
    pub nodes: Option<NodesView>,
    hourly_counts: Vec<u64>,
}

#[derive(Debug)]
pub struct NodesView {
    pub instance_types: Vec<InstanceTypeView>,
    pub nodes: Vec<NodeView>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct InstanceTypeView {
    pub name: String,
    pub count: usize,
    pub colour: usize,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeView {
    pub instance_type: String,
    pub zone: String,
    pub region: String,
    pub name: String,
}

impl NodesView {
    fn new(nodes: NodesSummary, colours: &HashMap<String, usize>) -> Self {
        let instance_types = nodes
            .by_instance_type
            .into_iter()
            .map(|(name, count)| InstanceTypeView {
                colour: colours.get(&name).copied().unwrap_or_default(),
                name,
                count,
            })
            .collect();

        let mut nodes: Vec<_> = nodes
            .nodes
            .into_iter()
            .map(|n| NodeView {
                name: n.name,
                instance_type: n.instance_type,
                region: n.region,
                zone: n.zone,
            })
            .collect();
        nodes.sort_unstable();

        Self {
            instance_types,
            nodes,
        }
    }
}

/// Assigns each instance type found in any cluster a palette index in alphabetical order so that
/// the same type always has the same colour regardless of which cluster is being displayed.
fn instance_type_colours(summary: &ClusterSummaryResponse) -> HashMap<String, usize> {
    summary
        .pools
        .iter()
        .flat_map(|p| &p.clusters)
        .filter_map(|c| c.nodes.as_ref())
        .flat_map(|n| n.by_instance_type.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, i % PALETTE_SIZE))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClaimView {
    pub run_id: Uuid,
    pub initiated_by: String,
    pub state: ClaimState,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClaimState {
    Owned,
    Reserved { executions_to_wait_for: usize },
}

impl ClaimView {
    pub fn label(&self) -> &'static str {
        match self.state {
            ClaimState::Owned => "Owned",
            ClaimState::Reserved { .. } => "Reserved",
        }
    }

    /// Only reservations are waiting on anything.
    pub fn executions_to_wait_for(&self) -> Option<usize> {
        match self.state {
            ClaimState::Owned => None,
            ClaimState::Reserved {
                executions_to_wait_for,
            } => Some(executions_to_wait_for),
        }
    }
}

impl From<ClusterClaimSummary> for ClaimView {
    fn from(claim: ClusterClaimSummary) -> Self {
        let (run_id, initiated_by, state) = match claim {
            ClusterClaimSummary::Owned {
                run_id,
                initiated_by,
            } => (run_id, initiated_by, ClaimState::Owned),
            ClusterClaimSummary::Acquiring {
                run_id,
                initiated_by,
            } => (run_id, initiated_by, ClaimState::Owned),
            ClusterClaimSummary::Reserved {
                run_id,
                initiated_by,
                executions_to_wait_for,
            } => (
                run_id,
                initiated_by,
                ClaimState::Reserved {
                    executions_to_wait_for,
                },
            ),
        };

        Self {
            run_id,
            initiated_by: initiated_by.unwrap_or_else(|| "unknown".to_owned()),
            state,
        }
    }
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
    instance_types: &'a [InstanceTypeView],
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

        let colours = instance_type_colours(&summary);
        let pools = summary
            .pools
            .into_iter()
            .map(|pool| {
                let queue = snapshot.pools.remove(&pool.name).unwrap_or_default();
                let queued_runs: HashSet<_> =
                    queue.pending_provisions.iter().map(|p| p.run_id).collect();

                PoolView {
                    supports_dedicated: pool.supports_dedicated,
                    queued_executions: queue.pending_provisions.len(),
                    queued_runs: queued_runs.len(),
                    queued_dedicated: queue
                        .pending_provisions
                        .iter()
                        .filter(|p| p.requires_dedicated)
                        .count(),
                    limits: limit_rows(&pool.per_user),
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
                                pending_other: queue.pending_non_provisions.len(),
                                claim: queue.claim.map(ClaimView::from),
                                config: config_rows(&cluster.execution),
                                nodes: cluster.nodes.map(|n| NodesView::new(n, &colours)),
                                hourly_counts: cluster
                                    .hourly_executions
                                    .iter()
                                    .map(|h| h.count)
                                    .collect(),
                                name: cluster.name,
                            }
                        })
                        .collect(),
                    name: pool.name,
                }
            })
            .collect();

        let queued = snapshot.summary.queued;
        let max_queued = summary.max_queued_executions;
        let queue_percent = percent(queued, max_queued);
        let queue_fill_modifier = match queue_percent {
            100.. => "usage-bar__fill--full",
            QUEUE_WARN_PERCENT.. => "usage-bar__fill--warn",
            _ => "",
        };

        Self {
            queued,
            max_queued,
            running: snapshot.summary.running,
            queue_percent,
            queue_fill_modifier,
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
                    instance_types: c.nodes.as_ref().map_or(&[], |n| &n.instance_types),
                })
                .collect(),
        }
    }
}

/// Clamped to 100 so that over-subscription still renders as a full bar.
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
    ]
}

fn limit_rows(pu: &PerUserExecutionSummary) -> Vec<(&'static str, String)> {
    vec![
        ("Max concurrent runs", pu.max_concurrent_runs.to_string()),
        ("Max queued runs", pu.max_queued_runs.to_string()),
        (
            "Max queued executions",
            pu.max_queued_executions.to_string(),
        ),
        ("Max runs per hour", pu.max_runs_per_hour.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::mocks::{
        sample_cluster_summary, sample_event_queue_snapshot, sample_nodes,
    };
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

    #[test_case(4, ""; "below warning threshold")]
    #[test_case(15, "usage-bar__fill--warn"; "at warning threshold")]
    #[test_case(20, "usage-bar__fill--full"; "at capacity")]
    #[test]
    fn queue_fill_modifier_reflects_queue_depth(queued: usize, expected: &str) {
        let mut snapshot = sample_event_queue_snapshot();
        snapshot.summary.queued = queued;

        let d = DashboardView::new(snapshot, sample_cluster_summary(), String::new());

        assert_eq!(d.queue_fill_modifier, expected);
    }

    #[test]
    fn queue_state_is_joined_onto_each_cluster() {
        let d = dashboard();
        let clusters: Vec<_> = d
            .pools
            .iter()
            .flat_map(|p| p.clusters.iter().map(move |c| (p.name.as_str(), c)))
            .map(|(pool, c)| (pool, c.name.as_str(), c.running, c.pending_other))
            .collect();

        assert_eq!(
            clusters,
            vec![("default", "alpha", 2, 1), ("dedicated", "beta", 1, 0)]
        );
    }

    #[test]
    fn queued_executions_are_joined_onto_each_pool() {
        let d = dashboard();
        let pools: Vec<_> = d
            .pools
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.queued_executions,
                    p.queued_runs,
                    p.queued_dedicated,
                )
            })
            .collect();

        assert_eq!(pools, vec![("default", 2, 1, 0), ("dedicated", 1, 1, 1)]);
    }

    #[test]
    fn pools_report_dedicated_support_and_per_user_limits_once() {
        let d = dashboard();

        assert_eq!(
            d.pools
                .iter()
                .map(|p| (p.name.as_str(), p.supports_dedicated))
                .collect::<Vec<_>>(),
            vec![("default", false), ("dedicated", true)]
        );
        assert!(d.pools.iter().all(|p| {
            p.limits
                .iter()
                .any(|(l, v)| *l == "Max queued runs" && v == "5")
        }));
        assert!(d.pools.iter().flat_map(|p| p.clusters.iter()).all(|c| {
            c.config
                .iter()
                .all(|(label, _)| !label.contains("per user"))
        }));
    }

    #[test_case(
        ClusterClaimSummary::Owned {
            run_id: Uuid::from_u128(1),
            initiated_by: Some("bob".into())
        },
        "bob",
        ClaimState::Owned;
        "owned"
    )]
    #[test_case(
        ClusterClaimSummary::Reserved {
            run_id: Uuid::from_u128(1),
            initiated_by: None,
            executions_to_wait_for: 3,
        },
        "unknown",
        ClaimState::Reserved { executions_to_wait_for: 3 };
        "reserved with unknown initiator"
    )]
    #[test]
    fn claims_are_converted_for_display(
        claim: ClusterClaimSummary,
        initiator: &str,
        state: ClaimState,
    ) {
        assert_eq!(
            ClaimView::from(claim),
            ClaimView {
                run_id: Uuid::from_u128(1),
                initiated_by: initiator.to_owned(),
                state,
            }
        );
    }

    #[test]
    fn claims_are_joined_onto_their_cluster() {
        let d = dashboard();
        let claims: Vec<_> = d
            .pools
            .iter()
            .flat_map(|p| p.clusters.iter())
            .map(|c| (c.name.as_str(), c.claim.as_ref().map(|c| &c.state)))
            .collect();

        assert_eq!(
            claims,
            vec![
                ("alpha", None),
                (
                    "beta",
                    Some(&ClaimState::Reserved {
                        executions_to_wait_for: 1
                    })
                )
            ]
        );
    }

    #[test]
    fn js_data_includes_the_instance_types_for_each_cluster() {
        let mut summary = sample_cluster_summary();
        summary.pools[0].clusters[0].nodes = Some(sample_nodes(&[("a1", "m5.large", "z1")]));
        summary.pools[1].clusters[0].nodes = None;

        let d = DashboardView::new(EventQueueSnapshot::default(), summary, String::new());
        let js = d.js_data();

        assert_eq!(
            js.clusters[0].instance_types,
            &[InstanceTypeView {
                name: "m5.large".to_owned(),
                count: 1,
                colour: 0
            }]
        );
    }

    #[test]
    fn clusters_missing_from_the_snapshot_have_an_empty_queue() {
        let d = DashboardView::new(
            EventQueueSnapshot::default(),
            sample_cluster_summary(),
            String::new(),
        );

        assert!(d.pools.iter().all(|p| p.queued_executions == 0));
        assert!(
            d.pools
                .iter()
                .flat_map(|p| p.clusters.iter())
                .all(|c| c.running == 0 && c.pending_other == 0 && c.claim.is_none())
        );
    }
}
