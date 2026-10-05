use crate::{
    config::WorkloadClusters,
    db::{ClusterId, PoolId, TestExecution, TestRun, UpdateHandle},
    event_loop::{
        inner::{ClusterClaim, EventQueueInner, InnerSnapshotState},
        shared::{Shared, SharedSnapshotState},
    },
    resolver::{self, ResolverError, ResolverInput},
    state::TestRunWithPayload,
};
use rtf_orchestrator_shared::{
    OutputCollectionResponse,
    event_queue::{ClusterClaimSummary, EventQueueSnapshot},
    test_plan::OrchestratorTestPlan,
};
use sqlx::PgConnection;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use uuid::Uuid;

/// Access to shared EventQueue state.
///
/// Used to atomically reserve space for queuing new test executions in the resolver task.
#[derive(Debug, Clone)]
pub struct EventQueueState {
    shared: Arc<Mutex<Shared>>,
    eq_inner: Arc<Mutex<EventQueueInner>>,
    tx_resolve: UnboundedSender<ResolverInput>,
    pool_clusters: HashMap<PoolId, Vec<ClusterId>>,
    dedicated_pools: HashSet<PoolId>,
}

impl EventQueueState {
    pub(super) fn new(
        cfg: &WorkloadClusters,
        shared: Arc<Mutex<Shared>>,
        eq_inner: Arc<Mutex<EventQueueInner>>,
        tx_resolve: UnboundedSender<ResolverInput>,
    ) -> Self {
        Self {
            shared,
            eq_inner,
            tx_resolve,
            pool_clusters: cfg.pool_clusters(),
            dedicated_pools: cfg.dedicated_pools(),
        }
    }

    pub(super) async fn with_shared<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Shared) -> T,
    {
        f(&mut *self.shared.lock().await)
    }

    /// Counts across all of the clusters in `pool`.
    pub async fn user_queue_counts(&self, user: &str, pool: &PoolId) -> UserQueueCounts {
        let clusters = self
            .pool_clusters
            .get(pool)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let (queued, running) = {
            let inner = self.eq_inner.lock().await;
            let queued: Vec<Uuid> = inner.pending_provisions_for_pool(pool).collect();
            let running: HashSet<Uuid> = clusters
                .iter()
                .flat_map(|c| inner.execution_ids_for_cluster(c))
                .copied()
                .collect();

            (queued, running)
        };

        self.with_shared(|shared| {
            let run_of = |ex: &Uuid| shared.executions.get(ex).map(|e| e.run_uuid);
            let is_users = |run: &Uuid| {
                shared
                    .runs
                    .get(run)
                    .is_some_and(|r| r.initiated_by.as_deref() == Some(user))
            };

            let ongoing_runs: HashSet<Uuid> = running
                .iter()
                .filter_map(run_of)
                .filter(|run| is_users(run))
                .collect();

            let queued_exececutions: Vec<Uuid> = queued
                .iter()
                .copied()
                .filter(|ex| run_of(ex).is_some_and(|run| is_users(&run)))
                .collect();

            let queued_runs: HashSet<Uuid> = queued_exececutions
                .iter()
                .filter_map(run_of)
                .filter(|run| !ongoing_runs.contains(run))
                .collect();

            UserQueueCounts {
                ongoing_runs: ongoing_runs.len(),
                queued_runs: queued_runs.len(),
                queued_executions: queued_exececutions.len(),
            }
        })
        .await
    }

    pub fn supports_dedicated(&self, pool: &str) -> bool {
        self.dedicated_pools.iter().any(|p| p.as_str() == pool)
    }

    pub fn available_pools(&self) -> Vec<PoolId> {
        let mut pool_ids: Vec<_> = self.pool_clusters.keys().cloned().collect();
        pool_ids.sort_unstable();

        pool_ids
    }

    pub async fn event_queue_snapshot(&self) -> EventQueueSnapshot {
        let InnerSnapshotState {
            mut summary,
            claims,
            pools,
            mut clusters,
        } = self.eq_inner.lock().await.snapshot_state();

        self.with_shared(|shared| {
            let initiator = |run: &Uuid| shared.runs.get(run).and_then(|r| r.initiated_by.clone());

            for (cid, claim, running) in claims.into_iter() {
                clusters.entry(cid).or_default().claim = Some(match claim {
                    ClusterClaim::Reserved(run_id) => ClusterClaimSummary::Reserved {
                        run_id,
                        initiated_by: initiator(&run_id),
                        executions_to_wait_for: running,
                    },
                    ClusterClaim::Acquiring(run_id) => ClusterClaimSummary::Acquiring {
                        run_id,
                        initiated_by: initiator(&run_id),
                    },
                    ClusterClaim::Owned(run_id) => ClusterClaimSummary::Owned {
                        run_id,
                        initiated_by: initiator(&run_id),
                    },
                });
            }

            let SharedSnapshotState {
                cached_run_payloads,
                active_run_executions,
                resolved_execution_cache,
            } = shared.snapshot_state();

            summary.queued = shared.n_queued;

            EventQueueSnapshot {
                summary,
                pools,
                clusters,
                cached_run_payloads,
                active_run_executions,
                resolved_execution_cache,
            }
        })
        .await
    }

    /// Attempt to submit a [TestRunWithPayload] through to the resolver task if we are able to
    /// obtain sufficient pending execution claims.
    pub async fn try_submit_test_plan(
        &self,
        claim: Claim,
        trp: TestRunWithPayload,
    ) -> Result<(), SubmitError> {
        let n = trp.payload.test_plan.matrix.n_variants();
        if claim.0 != n {
            return Err(SubmitError::InvalidClaim);
        }

        match self.tx_resolve.send(ResolverInput::TestRun(Box::new(trp))) {
            Ok(_) => Ok(()),
            Err(_) => {
                // If we hit this branch then the channel is closed and we are likely shutting
                // down. But, we still attempt to be good citizens and release our claim on the
                // resolver queue to ensure that the shared state is correct.
                self.with_shared(|shared| shared.n_queued -= n).await;

                Err(SubmitError::ResolveChannelClosed)
            }
        }
    }

    /// Attempt to reserve the requested number of executions if there is capacity.
    ///
    /// Returns `true` if the claim was successful, otherwise `false`.
    pub async fn try_reserve_pending_executions(&self, tp: &OrchestratorTestPlan) -> Option<Claim> {
        let n = tp.matrix.n_variants();

        self.with_shared(|shared| {
            if shared.n_queued.saturating_add(n) <= shared.max_queued_executions {
                shared.n_queued += n;
                Some(Claim(n))
            } else {
                None
            }
        })
        .await
    }

    /// Return `n` queued execution claims back to the shared state.
    pub async fn release_pending_execution_claim(&self, claim: Claim) {
        self.with_shared(|shared| shared.n_queued -= claim.0).await;
    }

    pub(crate) async fn resolve_environment_for_execution(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<String> {
        self.with_shared(|shared| {
            shared
                .executions
                .get(&ex.uuid())
                .and_then(|e| e.resolved_config.as_ref())
                .map(|c| c.env_yaml.clone())
                .ok_or(ResolverError::UnknownExecution(ex.uuid()))
        })
        .await
    }

    pub(crate) async fn resolve_scenario_for_execution(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<String> {
        self.with_shared(|shared| {
            shared
                .executions
                .get(&ex.uuid())
                .and_then(|e| e.resolved_config.as_ref())
                .map(|c| c.scenario_yaml.clone())
                .ok_or(ResolverError::UnknownExecution(ex.uuid()))
        })
        .await
    }

    pub(crate) async fn resolve_output_collection_for_execution(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<OutputCollectionResponse> {
        self.with_shared(|shared| {
            shared
                .executions
                .get(&ex.uuid())
                .and_then(|e| e.resolved_config.as_ref())
                .map(|c| c.output_collection.clone())
                .ok_or(ResolverError::UnknownExecution(ex.uuid()))
        })
        .await
    }

    pub async fn purge_run(
        &self,
        tr: TestRun,
        admin_email: String,
        conn: &mut PgConnection,
    ) -> Option<()> {
        // Lock everything to prevent further work being done while we purge state
        let mut shared = self.shared.lock().await;
        let mut inner = self.eq_inner.lock().await;
        let run_id = tr.uuid();

        let run = shared.runs.remove(&run_id)?;
        conn.mark_run_as_cancelled(&tr, format!("cancelled by {admin_email}"))
            .await;
        conn.clear_cached_payload_for_run(run_id).await;

        let mut last_purged = None;
        for ex_id in run.executions.into_iter() {
            if let Ok(Some(ex)) = TestExecution::get_by_uuid(&ex_id, conn).await {
                purge_execution_inner(ex.clone(), &mut shared, &mut inner, conn).await;
                last_purged = Some(ex);
            }
        }

        if let Some(ex) = last_purged {
            inner.schedule_cluster_release(run_id, &ex);
        }

        None
    }

    pub async fn purge_execution(
        &self,
        ex: TestExecution,
        admin_email: String,
        conn: &mut PgConnection,
    ) -> Option<()> {
        // Lock everything to prevent further work being done while we purge state
        let mut shared = self.shared.lock().await;
        let mut inner = self.eq_inner.lock().await;

        conn.mark_execution_as_cancelled(&ex, format!("cancelled by {admin_email}"))
            .await;

        purge_execution_inner(ex, &mut shared, &mut inner, conn).await
    }
}

pub(super) async fn purge_execution_inner(
    ex: TestExecution,
    shared: &mut Shared,
    inner: &mut EventQueueInner,
    conn: &mut impl UpdateHandle,
) -> Option<()> {
    let ex_id = ex.uuid();

    inner.purge_execution_state(ex.clone());

    // Once the queue itself is tidied up, purge any remaining cached data we have
    let run_id = shared.executions.remove(&ex_id)?.run_uuid;
    let run = shared.runs.get_mut(&run_id)?;
    run.executions.remove(&ex_id);

    if run.executions.is_empty() {
        shared.runs.remove(&run_id);
        conn.clear_cached_payload_for_run(run_id).await;
        inner.schedule_cluster_release(run_id, &ex);
    }

    None
}

/// Errors that can be encountered when attempting to submit a test plan to the resolver task.
#[derive(Debug)]
pub enum SubmitError {
    /// The size of the provided claim did not match the number of test plan executions submitted
    InvalidClaim,
    /// Resolver channel is closed
    ResolveChannelClosed,
}

/// A claim for resolving a specific number of test executions.
///
/// This type is deliberately opaque so the owner is only able to pass it back to
/// [EventQueueState::try_submit_test_plan].
pub struct Claim(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserQueueCounts {
    pub ongoing_runs: usize,
    pub queued_runs: usize,
    pub queued_executions: usize,
}

impl UserQueueCounts {
    pub fn new(ongoing_runs: usize, queued_runs: usize, queued_executions: usize) -> Self {
        Self {
            ongoing_runs,
            queued_runs,
            queued_executions,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::{
        config::{Config, DEFAULT_POOL},
        context::OrchestratorContext,
        event_loop::{EventQueue, PendingProvision, QueueEvent, tests::stub_test_plan},
    };
    use rtf_config::templating::Scalar;
    use rtf_orchestrator_shared::payload::SourceKeyedArrayMap;
    use simple_test_case::test_case;

    fn empty_source_map<T>() -> SourceKeyedArrayMap<T> {
        SourceKeyedArrayMap {
            keys: vec![],
            data: vec![],
        }
    }

    impl EventQueueState {
        async fn with_inner<F, T>(&self, f: F) -> T
        where
            F: FnOnce(&mut EventQueueInner) -> T,
        {
            f(&mut *self.eq_inner.lock().await)
        }

        async fn initiator_for_run(&self, run_uuid: Uuid) -> Option<String> {
            self.with_shared(|shared| shared.runs.get(&run_uuid)?.initiated_by.clone())
                .await
        }
    }

    #[test_case(&[(1, true)]; "single claim below max")]
    #[test_case(&[(5, true)]; "single claim at max")]
    #[test_case(&[(9, false)]; "single claim above max")]
    #[test_case(&[(5, true), (1, false)]; "second claim after max")]
    #[test_case(&[(3, true), (3, false)]; "second claim would exceed max")]
    #[test_case(&[(3, true), (2, true), (1, false)]; "seq to max then over")]
    #[tokio::test]
    async fn try_reserve_pending_executions_returns_expected_value(claims: &[(usize, bool)]) {
        let mut cfg = Config::for_test().workload_clusters;
        cfg.max_queued_executions = 5;
        let (_, _, state, _) = EventQueue::new(&cfg);

        for (i, &(n, expected)) in claims.iter().enumerate() {
            let mut tp = stub_test_plan();
            tp.matrix.dimensions = HashMap::from([("a".to_string(), vec![Scalar::Bool(true); n])]);

            let successful = state.try_reserve_pending_executions(&tp).await;
            assert_eq!(successful.is_some(), expected, "claim {i}");
        }
    }

    #[tokio::test]
    async fn initiated_by_for_run_returns_the_recorded_initiator() {
        let cfg = Config::for_test();
        let (_, ph, state, _) = EventQueue::new(&cfg.workload_clusters);
        let run_uuid = Uuid::new_v4();

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        ph.cache_for_test_run(
            run_uuid,
            Some("alice@example.com".to_string()),
            Default::default(),
            false,
            ctx,
            stub_test_plan(),
        )
        .await;

        assert_eq!(
            state.initiator_for_run(run_uuid).await,
            Some("alice@example.com".to_string())
        );
    }

    #[tokio::test]
    async fn initiated_by_for_run_returns_none_for_an_unknown_run() {
        let (_, _, state, _) = EventQueue::new(&WorkloadClusters::for_test());

        assert_eq!(state.initiator_for_run(Uuid::new_v4()).await, None);
    }

    #[test_case(&[], UserQueueCounts::new(0, 0, 0); "nothing queued at all")]
    #[test_case(&[("alice", "default", &[false])], UserQueueCounts::new(0, 1, 1); "single execution")]
    #[test_case(&[("alice", "default", &[true])], UserQueueCounts::new(1, 0, 0); "single ongoing run")]
    #[test_case(&[("alice", "default", &[false, false])], UserQueueCounts::new(0, 1, 2); "one run with two queued executions")]
    #[test_case(&[("alice", "default", &[true, false])], UserQueueCounts::new(1, 0, 1); "single ongoing execution marks run ongoing")]
    #[test_case(&[("alice", "b", &[false])], UserQueueCounts::new(0, 0, 0); "runs for another pool are ignored")]
    #[test_case(&[("alice", "default", &[false]), ("bob", "default", &[false])], UserQueueCounts::new(0, 1, 1); "runs for another user are ignored")]
    #[tokio::test]
    async fn user_queue_counts_reflects_live_queue_state(
        queued_executions: &[(&str, &str, &[bool])],
        expected: UserQueueCounts,
    ) {
        let (_, ph, state, _) = EventQueue::new(
            &WorkloadClusters::for_test_with_available_clusters(10, "a", &["a", "b"]),
        );

        let cfg = Config::for_test();
        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );

        let mut ex_id = 1;
        for (user, pool, statuses) in queued_executions.iter() {
            let run_uuid = Uuid::new_v4();
            let cluster = ClusterId::new(if *pool == "default" { "a" } else { pool });
            let pool = PoolId::new(*pool);

            ph.cache_for_test_run(
                run_uuid,
                Some(user.to_string()),
                Default::default(),
                false,
                ctx.clone(),
                stub_test_plan(),
            )
            .await;

            for (i, &is_running) in statuses.iter().enumerate() {
                let ex = TestExecution::create_stub(ex_id + i as i32, 1, i, "test");
                state
                    .with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
                    .await;

                if is_running {
                    state
                        .with_inner(|inner| {
                            inner.insert_running_execution(ex.uuid(), cluster.clone())
                        })
                        .await
                } else {
                    state
                        .with_inner(|inner| {
                            inner.push_event(QueueEvent::Provision(
                                pool.clone(),
                                PendingProvision::new(ex, run_uuid, false),
                            ))
                        })
                        .await
                }
            }

            ex_id += statuses.len() as i32;
        }

        assert_eq!(
            state.user_queue_counts("alice", &DEFAULT_POOL).await,
            expected
        );
    }

    #[tokio::test]
    async fn user_queue_counts_sums_running_executions_across_the_clusters_in_a_pool() {
        let mut clusters = WorkloadClusters::for_test_with_available_clusters(10, "a", &["a", "b"]);
        clusters.cluster_pools.default.available_clusters = vec!["a".into(), "b".into()];
        clusters.cluster_pools.additional.clear();

        let (_, ph, state, _) = EventQueue::new(&clusters);
        let ctx = OrchestratorContext::new_from_inlined_files(
            &Config::for_test(),
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        let run_uuid = Uuid::new_v4();
        ph.cache_for_test_run(
            run_uuid,
            Some("alice".to_string()),
            Default::default(),
            false,
            ctx,
            stub_test_plan(),
        )
        .await;

        for (i, cluster) in ["a", "b"].into_iter().enumerate() {
            let ex = TestExecution::create_stub(i as i32 + 1, 1, i, "test");
            state
                .with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
                .await;
            state
                .with_inner(|inner| {
                    inner.insert_running_execution(ex.uuid(), ClusterId::new(cluster))
                })
                .await;
        }

        assert_eq!(
            state.user_queue_counts("alice", &DEFAULT_POOL).await,
            UserQueueCounts::new(1, 0, 0)
        );
    }

    #[tokio::test]
    async fn resolve_environment_for_execution_returns_error_when_cache_empty() {
        let (_, _, eqs, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eqs.resolve_environment_for_execution(&ex).await;

        assert_matches!(
            res,
            Err(ResolverError::UnknownExecution(_)),
            "expected UnknownExecution, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn resolve_scenario_for_execution_returns_error_when_cache_empty() {
        let (_, _, eqs, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eqs.resolve_scenario_for_execution(&ex).await;

        assert_matches!(
            res,
            Err(ResolverError::UnknownExecution(_)),
            "expected UnknownExecution, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn resolve_output_collection_for_execution_returns_error_when_cache_empty() {
        let (_, _, eqs, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eqs.resolve_output_collection_for_execution(&ex).await;

        assert_matches!(
            res,
            Err(ResolverError::UnknownExecution(_)),
            "expected UnknownExecution, got: {res:?}"
        );
    }
}
