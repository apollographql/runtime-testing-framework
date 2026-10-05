use crate::{
    config::{Config, WorkloadClusters},
    context::OrchestratorContext,
    db::{self, ClusterId, PoolId, Status, StatusTracked, TestExecution, TestRun, UpdateHandle},
    event_loop::{
        Event, EventData, PendingProvision, QueueEvent,
        eq_state::{EventQueueState, purge_execution_inner},
        inner::{ClusterClaim, EventQueueInner},
        prov_handle::ProvisioningHandle,
        shared::Shared,
    },
    resolver::{self, ResolverError, ResolverInput},
};
use rtf_orchestrator_shared::{payload::PreparedPayload, test_plan::OrchestratorEnvironment};
use sqlx::PgConnection;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{
    Mutex,
    mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel},
};
use tracing::{error, warn};
use uuid::Uuid;

const MSG_RESTART_CONFLICT: &str = "dedicated cluster in use by multiple runs on server restart";

/// Coordinates queuing of k8s events to provide back pressure and prioritise running executions
/// over newly submitted ones.
///
/// Held by the event loop task with paired [ProvisioningHandle] and [EventQueueState] structs that
/// are used elsewhere in the codebase to submit events to the queue and introspect the current
/// queue state.
#[derive(Debug)]
pub struct EventQueue {
    /// Sender for submitting events back to the queue
    tx: UnboundedSender<QueueEvent>,
    /// Receiver for accepting new events
    rx: UnboundedReceiver<QueueEvent>,
    /// Shared state between the queue and paired structs
    shared: Arc<Mutex<Shared>>,
    /// The inner state of the event queue itself.
    /// Shared so the admin endpoint can view the state
    inner: Arc<Mutex<EventQueueInner>>,
    /// Sender for forwarding resolve events to the resolver_task
    tx_resolve: UnboundedSender<ResolverInput>,
}

impl EventQueue {
    /// Construct a new [EventQueue] along with its paired [ProvisioningHandle] and [EventQueueState]
    /// structs.
    pub fn new(
        cfg: &WorkloadClusters,
    ) -> (
        Self,
        ProvisioningHandle,
        EventQueueState,
        UnboundedReceiver<ResolverInput>,
    ) {
        let shared = Arc::new(Mutex::new(Shared {
            runs: HashMap::new(),
            executions: HashMap::new(),
            max_queued_executions: cfg.max_queued_executions,
            n_queued: 0,
        }));
        let (tx_resolve, rx_resolve) = unbounded_channel();
        let (tx, rx) = unbounded_channel();

        let eq = EventQueue {
            tx,
            rx,
            shared,
            inner: Arc::new(Mutex::new(EventQueueInner::new(cfg))),
            tx_resolve: tx_resolve.clone(),
        };

        let ph = eq.provisioning_handle();
        let eqs = EventQueueState::new(cfg, eq.shared.clone(), eq.inner.clone(), tx_resolve);

        (eq, ph, eqs, rx_resolve)
    }

    pub fn tx(&self) -> UnboundedSender<QueueEvent> {
        self.tx.clone()
    }

    fn provisioning_handle(&self) -> ProvisioningHandle {
        ProvisioningHandle::new(self.shared.clone(), self.tx.clone())
    }

    pub(super) async fn with_shared<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Shared) -> T,
    {
        f(&mut *self.shared.lock().await)
    }

    pub(super) async fn with_inner<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut EventQueueInner) -> T,
    {
        f(&mut *self.inner.lock().await)
    }

    pub async fn is_empty(&self) -> bool {
        self.rx.is_empty() && self.with_inner(|inner| inner.is_empty()).await
    }

    #[inline(always)]
    async fn push_event(&mut self, evt: QueueEvent) {
        self.with_inner(|inner| inner.push_event(evt)).await
    }

    fn still_have_external_senders(&self) -> bool {
        self.rx.sender_strong_count() > 1
    }

    /// Returns the next [Event] to be processed, prioritising ongoing events over provisioning new
    /// namespaces.
    ///
    /// We buffer events internally and fully drain the channel of any events received since the
    /// last call to `next_event`. This method blocks when there are no internally buffered events
    /// and the channel is currently empty or if every cluster is running `max_concurrent_executions`
    /// and the only queued events would provision a new namespace.
    ///
    /// Returns [None] when all external senders have been dropped and the internal queues
    /// are empty.
    pub async fn next_event(&mut self) -> Option<Event> {
        loop {
            while self.still_have_external_senders() {
                match self.rx.try_recv() {
                    Ok(evt) => self.push_event(evt).await,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => unreachable!("we hold a sender"),
                }
            }

            if let Some(evt) = self.with_inner(EventQueueInner::next_event).await {
                return Some(evt);
            }

            if self.still_have_external_senders() {
                let evt = self.rx.recv().await?;
                self.push_event(evt).await;
            } else {
                // all external senders gone so event stream is now closed
                return None;
            }
        }
    }

    /// Remove the given execution from the running set, freeing a namespace slot for the
    /// next pending provision event, and decrement the parent run's outstanding-execution
    /// count.
    ///
    /// Returns `Some(run_uuid)` if this was the final execution for its parent run, otherwise
    /// `None`.
    pub async fn mark_execution_complete(&mut self, ex_id: Uuid) -> Option<Uuid> {
        if !self.inner.lock().await.remove_running_execution(ex_id) {
            warn!(%ex_id, "mark_execution_complete called for unknown execution id");
        }

        self.with_shared(|shared| {
            let run_uuid = shared.executions.remove(&ex_id)?.run_uuid;
            let run = shared.runs.get_mut(&run_uuid)?;
            run.executions.remove(&ex_id);

            if run.executions.is_empty() {
                shared.runs.remove(&run_uuid);
                Some(run_uuid)
            } else {
                None
            }
        })
        .await
    }

    pub(super) async fn abort_run_acquiring_cluster(
        &mut self,
        run_uuid: Uuid,
        ex: &TestExecution,
        msg: String,
        conn: &mut impl UpdateHandle,
    ) {
        let mut shared = self.shared.lock().await;
        let mut inner = self.inner.lock().await;

        let mut aborted = inner.abort_pending_provisions_for(run_uuid);
        aborted.push(ex.clone());

        for ex in aborted.iter() {
            conn.mark_execution_as_unrunnable(ex, msg.clone()).await;
            inner.remove_running_execution(ex.uuid());
            shared.executions.remove(&ex.uuid());
        }

        shared.runs.remove(&run_uuid);
        inner.schedule_cluster_release(run_uuid, ex);
        conn.clear_cached_payload_for_run(run_uuid).await;
    }

    pub(crate) async fn scenario_job_params(&self, ex_id: Uuid) -> Option<ScenarioJobParams> {
        self.with_shared(|shared| {
            let ex_state = shared.executions.get(&ex_id)?;
            let ex_cfg = ex_state.resolved_config.as_ref()?;
            let allow_k8s_write = shared
                .runs
                .get(&ex_state.run_uuid)?
                .workload_config
                .allow_k8s_write;

            Some(ScenarioJobParams {
                docker_image: ex_cfg.docker_image.clone(),
                command: ex_cfg.docker_command.clone(),
                allow_k8s_write,
            })
        })
        .await
    }

    pub(crate) async fn resolved_environment_for_execution(
        &self,
        ex_id: Uuid,
    ) -> Option<OrchestratorEnvironment> {
        self.with_shared(|shared| {
            shared
                .executions
                .get(&ex_id)
                .and_then(|e| e.resolved_config.as_ref())
                .map(|c| c.environment.clone())
        })
        .await
    }

    /// Send a [ResolverInput] to the resolver_task channel.
    pub(crate) fn send_to_resolver(&self, input: ResolverInput) -> resolver::Result<()> {
        self.tx_resolve
            .send(input)
            .map_err(|_| ResolverError::EventChannelClosed)
    }

    /// Initialise the in-memory queue state based on current DB cache data.
    pub(crate) async fn init_queue_state(
        &mut self,
        cfg: &Config,
        conn: &mut PgConnection,
    ) -> crate::Result<()> {
        let cache = try_load_payload_cache(conn).await?;

        self.init_queue_state_from_cache(cache, cfg, conn).await
    }

    async fn init_queue_state_from_cache(
        &mut self,
        cache: HashMap<Uuid, (TestRun, PreparedPayload)>,
        cfg: &Config,
        conn: &mut impl UpdateHandle,
    ) -> crate::Result<()> {
        let h = self.provisioning_handle();
        let mut recovered_runs: HashMap<Uuid, (TestRun, Vec<TestExecution>)> = HashMap::new();

        for (run_uuid, (tr, payload)) in cache.into_iter() {
            let pool = tr.workload_pool();
            let fallback_cluster = self
                .with_inner(|inner| inner.fallback_cluster_for_pool(pool.clone()))
                .await?;

            let requires_dedicated = conn.run_requires_dedicated_cluster(&tr).await?;
            let workload_config = conn.run_workload_config(&tr).await?;
            let PreparedPayload {
                test_plan,
                relative_files,
                custom_providers,
                variable_sources,
                ..
            } = payload;
            let ctx = OrchestratorContext::new_from_inlined_files(
                cfg,
                relative_files,
                custom_providers,
                variable_sources,
            );
            h.cache_for_test_run(
                run_uuid,
                tr.initiated_by().map(|s| s.to_owned()),
                workload_config,
                requires_dedicated,
                ctx,
                test_plan,
            )
            .await;

            let executions = conn.executions_for_run(&tr).await?;
            let mut n_in_flight = 0;
            let mut live = Vec::new();

            for ex in executions.into_iter() {
                let res = self
                    .try_recover_execution(
                        &ex,
                        run_uuid,
                        &pool,
                        &fallback_cluster,
                        requires_dedicated,
                        &h,
                        conn,
                    )
                    .await?;
                let recovered = match res {
                    Some(recovered) => recovered,
                    None => continue,
                };

                live.push(ex);
                for evt in recovered.into_iter() {
                    let _ = self.tx.send(evt);
                    n_in_flight += 1;
                }
            }

            if n_in_flight == 0 {
                // Nothing left to do for this run so evict from the cache
                h.evict_cached_run_state(run_uuid).await;
                conn.clear_cached_payload_for_run(run_uuid).await;
            } else {
                recovered_runs.insert(run_uuid, (tr, live));
            }
        }

        self.purge_conflicting_dedicated_runs(recovered_runs, conn)
            .await;

        Ok(())
    }

    /// Clusters from pools supporting dedicated have three valid states:
    ///   1. No ongoing executions
    ///   2. Ongoing executions from 1 or more runs that do not required a dedicated cluster
    ///   3. Ongoing executions from a single run that requires a dedicated cluster
    ///
    /// Following a restart, any clusters in a dedicated pool that are NOT in one of these states
    /// are invalid and we purge the cluster entirely.
    ///
    /// This should only be possible due to operator error by altering cluster config while
    /// clusters are in use. If we have hit this error state and that WASN'T the case we have an
    /// unexpected edge case in the restart logic that needs to be investigated!
    async fn purge_conflicting_dedicated_runs(
        &mut self,
        recovered_runs: HashMap<Uuid, (TestRun, Vec<TestExecution>)>,
        conn: &mut impl UpdateHandle,
    ) {
        let mut shared = self.shared.lock().await;
        let mut inner = self.inner.lock().await;

        let mut conflicting_runs = HashSet::new();
        for (cluster, claim) in inner.iter_claims() {
            if !matches!(claim, ClusterClaim::Owned(_)) {
                continue;
            }

            let runs: HashSet<Uuid> = inner
                .execution_ids_for_cluster(cluster)
                .filter_map(|ex| shared.executions.get(ex).map(|e| e.run_uuid))
                .collect();

            if runs.len() > 1 {
                error!(%cluster, ?runs, "invalid dedicated cluster restart state");
                conflicting_runs.extend(runs);
            }
        }

        for run_uuid in conflicting_runs.into_iter() {
            let (tr, executions) = match recovered_runs.get(&run_uuid) {
                Some(details) => details,
                None => continue,
            };

            error!(%run_uuid, "purging run as part of invalid cluster state following restart");
            conn.mark_run_as_unrunnable(tr, MSG_RESTART_CONFLICT.into())
                .await;

            for ex in executions.iter() {
                conn.mark_execution_as_unrunnable(ex, MSG_RESTART_CONFLICT.into())
                    .await;
                purge_execution_inner(ex.clone(), &mut shared, &mut inner, conn).await;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn try_recover_execution(
        &mut self,
        ex: &TestExecution,
        run_uuid: Uuid,
        pool: &PoolId,
        fallback_cluster: &ClusterId,
        requires_dedicated: bool,
        h: &ProvisioningHandle,
        conn: &mut impl UpdateHandle,
    ) -> crate::Result<Option<Vec<QueueEvent>>> {
        let current = match conn.try_current_test_execution_status(ex).await? {
            Some(s) if s.status.is_terminal() => return Ok(None),
            None => Status::Initialising,
            Some(s) => s.status,
        };

        let ex_uuid = ex.uuid();

        self.with_shared(|shared| shared.register_execution(ex_uuid, run_uuid))
            .await;

        let cluster = ex
            .workload_cluster()
            .unwrap_or_else(|| fallback_cluster.clone());

        if current > Status::Resolving {
            self.with_inner(|inner| {
                inner.insert_running_execution(ex_uuid, cluster.clone());
                // This insert is potentially invalid but we make it now to allow us to identify
                // all executions that are in an invalid dedicated state after processing
                // everything we found in-flight.
                if requires_dedicated {
                    inner.mark_cluster_as_owned(&cluster, run_uuid);
                }
            })
            .await;
        }

        let data = match current {
            Status::Successful | Status::Failed | Status::Unrunnable | Status::Cancelled => {
                unreachable!("is_terminal() checked above")
            }

            // No side-effecting actions taken yet so we're clear to run the full event flow.
            Status::Initialising | Status::Resolving => {
                return Ok(Some(vec![QueueEvent::Provision(
                    pool.clone(),
                    PendingProvision::new(ex.clone(), run_uuid, requires_dedicated),
                )]));
            }

            // This execution would have previously claimed a running execution slot but we don't
            // know how far through the provisioning process we were. So we mark it as running to
            // reserve the slot and then run the full event flow.
            // This will resolve the config before attempting to create the workflow which is
            // idempotent, allowing us to skip straight to waiting for the workflow to complete if
            // needed.
            Status::Provisioning => match h.resolve_and_cache_config(ex).await {
                Ok(_) => vec![EventData::CreateEnvArgoWorkflow],
                Err(e) => {
                    warn!(%ex_uuid, %e, "failed to resolve config for in-flight Provisioning execution");
                    vec![
                        EventData::MarkUnrunnable(e.to_string()),
                        EventData::CleanupNamespace,
                    ]
                }
            },

            // We should have a live namespace but we don't know if the scenario was part way
            // through starting up or not triggered yet. As with Status::Provisioning we resolve
            // the config before attempting to create the job which is also idempotent, again
            // allowing us to skip straight to waiting for the job to complete if needed.
            Status::EnvironmentReady => match h.resolve_and_cache_config(ex).await {
                Ok(_) => vec![EventData::CreateScenarioJob],
                Err(e) => {
                    warn!(%ex_uuid, %e, "failed to resolve config for in-flight EnvironmentReady execution");
                    vec![
                        EventData::MarkUnrunnable(e.to_string()),
                        EventData::CleanupNamespace,
                    ]
                }
            },

            // Scenario in progress, so re-attach the k8s watcher and wait for it to complete.
            Status::Running => vec![EventData::WaitForScenarioJob],
        };

        Ok(Some(
            data.into_iter()
                .map(|data| QueueEvent::Other(Event::new(ex.clone(), cluster.clone(), data)))
                .collect(),
        ))
    }
}

/// Run and Execution specific state for building the Scenario k8s job definition
#[derive(Debug)]
pub struct ScenarioJobParams {
    pub docker_image: String,
    pub command: String,
    pub allow_k8s_write: bool,
}

/// Load our cached trigger payload state from the DB, evicting malformed payloads and marking
/// their incomplete child executions as unrunnable.
async fn try_load_payload_cache(
    conn: &mut PgConnection,
) -> db::Result<HashMap<Uuid, (TestRun, PreparedPayload)>> {
    let (cache, malformed_runs) = TestRun::load_payload_cache(conn).await?;

    for tr in malformed_runs.into_iter() {
        warn!(run_uuid=%tr.uuid(), "malformed payload cache entry");
        let executions = tr.executions(conn).await?;

        for ex in executions.iter() {
            if let Some(s) = ex.try_current_status(conn).await?
                && !s.status.is_terminal()
            {
                ex.set_status(
                    Status::Unrunnable,
                    Some("malformed cache state on server restart".into()),
                    conn,
                )
                .await?;
            }
        }

        TestRun::clear_cached_payload(tr.uuid(), conn).await?;
    }

    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, DEFAULT_POOL},
        conn,
        context::OrchestratorContext,
        db::{MockUpdateHandle, Queryable},
        event_loop::{
            shared::{ExecutionState, ResolvedExecutionConfig},
            tests::stub_test_plan,
        },
    };
    use rtf_config::formats::NullEnvironment;
    use rtf_orchestrator_shared::{
        OutputCollectionResponse, PrometheusQueries,
        event_queue::{ClusterClaimSummary, PendingProvisionSummary},
        payload::SourceKeyedArrayMap,
    };
    use simple_test_case::test_case;
    use std::{assert_matches, collections::HashMap, time::Duration};

    fn empty_source_map<T>() -> SourceKeyedArrayMap<T> {
        SourceKeyedArrayMap {
            keys: vec![],
            data: vec![],
        }
    }

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    fn perf_cluster() -> ClusterId {
        ClusterId::new("router_perf")
    }

    fn perf_pool() -> PoolId {
        PoolId::new("router_perf")
    }

    fn provision(pool: PoolId, ex: TestExecution) -> QueueEvent {
        QueueEvent::Provision(pool, PendingProvision::new(ex, Uuid::new_v4(), false))
    }

    #[tokio::test]
    async fn mark_execution_complete_updates_running_set() {
        let (mut eq, _, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let id = Uuid::new_v4();
        eq.with_inner(|inner| inner.insert_running_execution(id, alpha_cluster()))
            .await;

        let evicted = eq.mark_execution_complete(id).await;

        assert!(
            !eq.with_inner(|inner| inner
                .running_executions()
                .get(&alpha_cluster())
                .is_some_and(|running| running.contains(&id)))
                .await,
            "execution was still present in running set"
        );
        assert_eq!(
            evicted, None,
            "no run should be evicted when execution wasn't registered"
        );
    }

    #[tokio::test]
    async fn mark_execution_complete_evicts_cache_after_last_execution() {
        let cfg = Config::for_test();
        let (mut eq, h, _, _) = EventQueue::new(&cfg.workload_clusters);
        let run_uuid = Uuid::new_v4();
        let ex1 = Uuid::new_v4();
        let ex2 = Uuid::new_v4();

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        h.cache_for_test_run(
            run_uuid,
            None,
            Default::default(),
            false,
            ctx,
            stub_test_plan(),
        )
        .await;
        eq.with_shared(|shared| {
            shared.register_execution(ex1, run_uuid);
            shared.register_execution(ex2, run_uuid);
        })
        .await;
        eq.with_inner(|inner| {
            inner.insert_running_execution(ex1, alpha_cluster());
            inner.insert_running_execution(ex2, alpha_cluster());
        })
        .await;

        // Completing the first execution leaves the run with 1 open execution: no eviction.
        let evicted = eq.mark_execution_complete(ex1).await;
        assert_eq!(evicted, None);
        eq.with_shared(|shared| {
            assert_eq!(
                shared.runs.get(&run_uuid).map(|run| run.executions.len()),
                Some(1),
                "{:?}",
                shared.runs
            );
            assert!(shared.executions.contains_key(&ex2));
        })
        .await;

        let evicted = eq.mark_execution_complete(ex2).await;
        assert_eq!(evicted, Some(run_uuid),);

        eq.with_shared(|shared| {
            assert!(
                !shared.runs.contains_key(&run_uuid),
                "run entry should be cleared"
            );
            assert!(
                !shared.executions.contains_key(&ex2),
                "ex1 should be removed"
            );
            assert!(
                !shared.executions.contains_key(&ex1),
                "ex2 should be removed"
            );
        })
        .await;
    }

    async fn cache_stub_run(h: &ProvisioningHandle, run_uuid: Uuid) {
        let ctx = OrchestratorContext::new_from_inlined_files(
            &Config::for_test(),
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        h.cache_for_test_run(
            run_uuid,
            None,
            Default::default(),
            false,
            ctx,
            stub_test_plan(),
        )
        .await;
    }

    #[tokio::test]
    async fn next_event_gates_provisions_on_namespace_capacity() {
        // max concurrent of 1: any provision dispatch is blocked while a slot is in use.
        let (mut q, _h, _, _) = EventQueue::new(
            &WorkloadClusters::for_test_with_available_clusters(1, "alpha", &["alpha"]),
        );
        let blocking_id = Uuid::new_v4();
        q.with_inner(|inner| inner.insert_running_execution(blocking_id, alpha_cluster()))
            .await;

        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let ex_uuid = ex.uuid();
        q.tx.send(provision(DEFAULT_POOL, ex)).unwrap();

        let res = tokio::time::timeout(Duration::from_millis(50), q.next_event()).await;
        assert!(res.is_err(), "next_event should have blocked at capacity");

        let _ = q.mark_execution_complete(blocking_id).await;
        let evt = tokio::time::timeout(Duration::from_secs(1), q.next_event())
            .await
            .expect("next_event should have yielded within 1s")
            .expect("should have returned Some(event)");

        assert_eq!(evt.test_execution.uuid(), ex_uuid);
        assert_matches!(evt.data, EventData::ResolveConfig);

        q.with_inner(|inner| {
            assert!(
                inner
                    .running_executions()
                    .get(&alpha_cluster())
                    .is_some_and(|running| running.contains(&ex_uuid)),
                "running executions: {:?}",
                inner.running_executions()
            )
        })
        .await;
    }

    #[tokio::test]
    async fn next_event_round_robins_provisions_across_pools() {
        let (mut q, _h, _, _) =
            EventQueue::new(&WorkloadClusters::for_test_with_available_clusters(
                10,
                "alpha",
                &["alpha", "router_perf"],
            ));

        // Both of the default pool's events are pushed before either of router_perf's, so a
        // FIFO-only queue would dispatch them back-to-back; round-robin fairness should still
        // alternate pools.
        for (pool, ex_id) in [
            (DEFAULT_POOL, 1),
            (DEFAULT_POOL, 2),
            (perf_pool(), 3),
            (perf_pool(), 4),
        ] {
            q.push_event(provision(
                pool,
                TestExecution::create_stub(ex_id, 1, 0, "test"),
            ))
            .await;
        }

        let mut dispatched = Vec::new();
        for _ in 0..4 {
            let evt = tokio::time::timeout(Duration::from_secs(1), q.next_event())
                .await
                .expect("next_event should have yielded within 1s")
                .expect("should have returned Some(event)");
            dispatched.push((evt.cluster, evt.test_execution.id()));
        }

        assert_eq!(
            dispatched,
            vec![
                (alpha_cluster(), 1),
                (perf_cluster(), 3),
                (alpha_cluster(), 2),
                (perf_cluster(), 4),
            ],
            "expected pools to alternate rather than draining the default pool before router_perf"
        );
    }

    #[tokio::test]
    async fn next_event_skips_saturated_cluster_without_blocking_others() {
        let (mut q, _h, _, _) =
            EventQueue::new(&WorkloadClusters::for_test_with_available_clusters(
                1,
                "alpha",
                &["alpha", "router_perf"],
            ));

        // Saturate alpha's one namespace slot.
        let blocking_id = Uuid::new_v4();
        q.with_inner(|inner| inner.insert_running_execution(blocking_id, alpha_cluster()))
            .await;

        let alpha_ex = TestExecution::create_stub(1, 1, 0, "test");
        let perf_ex = TestExecution::create_stub(2, 1, 0, "test");
        let perf_ex_uuid = perf_ex.uuid();

        q.push_event(provision(DEFAULT_POOL, alpha_ex)).await;
        q.push_event(provision(perf_pool(), perf_ex)).await;

        let evt = tokio::time::timeout(Duration::from_secs(1), q.next_event())
            .await
            .expect("next_event should have yielded router_perf's event within 1s")
            .expect("should have returned Some(event)");

        assert_eq!(
            evt.test_execution.uuid(),
            perf_ex_uuid,
            "saturated alpha cluster should not block dequeuing router_perf's event"
        );
        assert_eq!(evt.cluster, perf_cluster());
    }

    fn provision_evt(ex_id: i32) -> QueueEvent {
        provision(
            DEFAULT_POOL,
            TestExecution::create_stub(ex_id, 1, 0, "test"),
        )
    }

    fn cleanup_evt(ex_id: i32) -> QueueEvent {
        QueueEvent::Other(Event::new(
            TestExecution::create_stub(ex_id, 1, 0, "test"),
            alpha_cluster(),
            EventData::CleanupNamespace,
        ))
    }

    #[test_case(vec![provision_evt(1), provision_evt(2)], 1; "only provision")]
    #[test_case(vec![cleanup_evt(1), cleanup_evt(2)], 1; "only non-provision")]
    #[test_case(vec![provision_evt(1), cleanup_evt(2)], 2; "both")]
    #[tokio::test]
    async fn next_event_returns_expected_event_from_pending(
        events: Vec<QueueEvent>,
        expected_id: i32,
    ) {
        // Handle needs to stay alive: dropping it reduces sender_strong_count to 1, causing the
        // drain loop to return None before reaching the priority selection logic.
        let (mut q, _h, _, _) = EventQueue::new(&WorkloadClusters::for_test());

        for evt in events.into_iter() {
            q.push_event(evt).await;
        }

        let evt = q.next_event().await.expect("should have returned an event");

        assert_eq!(
            evt.test_execution.id(),
            expected_id,
            "wrong event returned: {evt:?}"
        );
    }

    #[tokio::test]
    async fn next_event_drains_channel_before_selecting() {
        // Handle needs to stay alive: dropping it reduces sender_strong_count to 1, causing the
        // drain loop to return None before reaching the priority selection logic.
        let (mut q, _h, _, _) = EventQueue::new(&WorkloadClusters::for_test());

        // Send the provision event first so we know that we're not just relying on ordering
        q.tx.send(provision_evt(1)).unwrap();
        q.tx.send(cleanup_evt(2)).unwrap();

        let evt = q.next_event().await.expect("should have returned an event");

        assert_eq!(evt.test_execution.id(), 2, "wrong event returned: {evt:?}");
    }

    #[tokio::test]
    async fn next_event_blocks_when_no_events_are_available() {
        // Handle needs to stay alive: dropping it reduces sender_strong_count to 1, causing the
        // drain loop to return None before reaching the priority selection logic.
        let (mut q, _h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let tx = q.tx.clone();

        let next_event_task = tokio::spawn(async move { q.next_event().await });

        // yield to allow the next_event task to run (if able)
        tokio::task::yield_now().await;
        assert!(
            !next_event_task.is_finished(),
            "should be blocked waiting for an event"
        );

        tx.send(cleanup_evt(1)).unwrap();

        let evt = tokio::time::timeout(Duration::from_secs(1), next_event_task)
            .await
            .expect("should have unblocked within 1s")
            .expect("task should not have panicked")
            .expect("should have returned Some(event)");

        assert_eq!(evt.test_execution.id(), 1, "wrong event returned: {evt:?}");
    }

    #[tokio::test]
    async fn next_event_returns_none_when_no_external_senders_remain() {
        let (mut q, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());

        // Start with an event in the channel so we skip the blocking recv call and drop into the
        // drain loop.
        q.tx.send(cleanup_evt(1)).unwrap();
        drop(h);

        assert_eq!(q.next_event().await, None, "should have returned None");
        assert_eq!(
            q.inner.lock().await.pending_non_provisions().len(),
            0,
            "unexpected recv"
        );
        assert!(!q.rx.is_empty(), "event should still be in the channel");
    }

    #[tokio::test]
    async fn push_event_routes_provision_events_to_their_pool() {
        let (mut q, _h, _, _) = EventQueue::new(&WorkloadClusters::for_test());

        q.push_event(provision_evt(1)).await;

        q.with_inner(|inner| {
            assert_eq!(
                inner.pending_provisions().get(&DEFAULT_POOL).unwrap().len(),
                1
            );
            assert_eq!(inner.pending_non_provisions().len(), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn push_event_routes_ongoing_events_to_non_provisions() {
        let (mut q, _h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let evt = QueueEvent::Other(Event {
            test_execution: TestExecution::create_stub(1, 1, 0, "test"),
            cluster: alpha_cluster(),
            data: EventData::CreateEnvArgoWorkflow,
        });

        q.push_event(evt).await;

        q.with_inner(|inner| {
            assert!(!inner.pending_provisions().contains_key(&DEFAULT_POOL));
            assert_eq!(inner.pending_non_provisions().len(), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn resolve_and_cache_config_stores_yaml_and_docker_details() {
        let cfg = Config::for_test();
        let (eq, ph, _, _) = EventQueue::new(&cfg.workload_clusters);

        let run_uuid = Uuid::new_v4();
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        ph.cache_for_test_run(
            run_uuid,
            None,
            Default::default(),
            false,
            ctx,
            stub_test_plan(),
        )
        .await;
        eq.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
            .await;

        let ex_uuid = ex.uuid();

        let res = ph.resolve_and_cache_config(&ex).await;

        assert!(res.is_ok(), "{res:?}");
        eq.with_shared(|shared| {
            let cached = shared
                .executions
                .get(&ex_uuid)
                .and_then(|e| e.resolved_config.as_ref())
                .expect("execution config should be cached");

            assert!(!cached.env_yaml.is_empty(), "env YAML should be cached");
            assert!(
                !cached.scenario_yaml.is_empty(),
                "scenario YAML should be cached"
            );
            assert!(
                !cached.docker_image.is_empty(),
                "scenario docker image should be cached"
            );
            assert!(
                !cached.docker_command.is_empty(),
                "scenario docker command should be cached"
            );
            assert_matches!(
                cached.environment,
                OrchestratorEnvironment::DockerCompose(_),
                "expected the typed environment to be cached, got {:?}",
                cached.environment
            );
        })
        .await;
    }

    #[tokio::test]
    async fn resolve_and_cache_config_caches_null_environment_with_no_prometheus_queries() {
        let cfg = Config::for_test();
        let (eq, ph, _, _) = EventQueue::new(&cfg.workload_clusters);
        let run_uuid = Uuid::new_v4();
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let mut test_plan = stub_test_plan();
        test_plan.environment.execution =
            OrchestratorEnvironment::Null(NullEnvironment { skip: true });

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        ph.cache_for_test_run(run_uuid, None, Default::default(), false, ctx, test_plan)
            .await;
        eq.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
            .await;

        let res = ph.resolve_and_cache_config(&ex).await;
        assert!(res.is_ok(), "{res:?}");

        eq.with_shared(|shared| {
            let cached = shared
                .executions
                .get(&ex.uuid())
                .and_then(|e| e.resolved_config.as_ref())
                .expect("execution config should be cached");

            assert_matches!(
                cached.environment,
                OrchestratorEnvironment::Null(_),
                "expected a null environment to be cached, got {:?}",
                cached.environment
            );
            assert!(
                cached.output_collection.prometheus.environment.is_empty(),
                "expected no environment prometheus queries for a null environment"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn resolved_environment_for_execution_returns_cached_environment() {
        let (eq, _, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eq.resolved_environment_for_execution(ex.uuid()).await;
        assert!(
            res.is_none(),
            "expected no cached environment before resolution, got {res:?}"
        );

        eq.with_shared(|shared| {
            shared.executions.insert(
                ex.uuid(),
                ExecutionState {
                    run_uuid: Uuid::new_v4(),
                    resolved_config: Some(ResolvedExecutionConfig {
                        env_yaml: "yaml".to_string(),
                        scenario_yaml: "yaml".to_string(),
                        output_collection: OutputCollectionResponse {
                            execution_variables: String::new(),
                            prometheus: PrometheusQueries {
                                environment: Vec::new(),
                                scenario: Vec::new(),
                            },
                        },
                        docker_image: "image".to_string(),
                        docker_command: "command".to_string(),
                        environment: OrchestratorEnvironment::Null(NullEnvironment { skip: true }),
                    }),
                },
            );
        })
        .await;

        let res = eq.resolved_environment_for_execution(ex.uuid()).await;
        assert_matches!(
            res,
            Some(OrchestratorEnvironment::Null(_)),
            "expected the cached null environment to be returned, got {res:?}"
        );
    }

    #[tokio::test]
    async fn mark_execution_complete_evicts_resolved_execution_cache() {
        let (mut eq, _, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let run_uuid = Uuid::new_v4();
        let ex_id = Uuid::new_v4();

        eq.with_shared(|shared| {
            shared.register_execution(ex_id, run_uuid);
            shared.executions.get_mut(&ex_id).unwrap().resolved_config =
                Some(ResolvedExecutionConfig {
                    env_yaml: "yaml".to_string(),
                    scenario_yaml: "yaml".to_string(),
                    output_collection: OutputCollectionResponse {
                        execution_variables: String::new(),
                        prometheus: PrometheusQueries {
                            environment: Vec::new(),
                            scenario: Vec::new(),
                        },
                    },
                    docker_image: "image".to_string(),
                    docker_command: "command".to_string(),
                    environment: OrchestratorEnvironment::Null(NullEnvironment { skip: true }),
                });
        })
        .await;
        eq.with_inner(|inner| inner.insert_running_execution(ex_id, alpha_cluster()))
            .await;

        eq.mark_execution_complete(ex_id).await;

        eq.with_shared(|shared| {
            assert!(
                !shared.executions.contains_key(&ex_id),
                "execution entry should be evicted"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn send_to_resolver_forwards_resolve_env_config() {
        let (eq, _, _, mut rx) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        eq.send_to_resolver(ResolverInput::ResolveConfig(ex.clone(), alpha_cluster()))
            .unwrap();

        let input = rx.try_recv().expect("ResolverInput should be forwarded");
        assert_matches!(
            input,
            ResolverInput::ResolveConfig(ref e, _) if e.uuid() == ex.uuid(),
            "wrong input forwarded: {input:?}"
        );
    }

    fn stub_trigger_payload() -> PreparedPayload {
        PreparedPayload {
            variables: None,
            variable_sources: HashMap::default(),
            test_plan: stub_test_plan(),
            relative_files: SourceKeyedArrayMap {
                keys: vec![],
                data: vec![],
            },
            custom_providers: SourceKeyedArrayMap {
                keys: vec![],
                data: vec![],
            },
        }
    }

    async fn prepare_init_queue_test<const N: usize>(
        statuses: [Status; N],
    ) -> (EventQueue, MockUpdateHandle, TestRun, [TestExecution; N]) {
        let cfg = Config::for_test();
        let (mut eq, _, _, _) = EventQueue::new(&cfg.workload_clusters);

        let tr = TestRun::create_stub(1, "test");
        let mut c = MockUpdateHandle::with_run(tr.clone());

        let mut exs = Vec::with_capacity(statuses.len());

        for status in statuses.into_iter() {
            let ex = c.init_execution(&tr, "test", 0).await.unwrap();
            c.update_test_execution_status(&ex, status, None)
                .await
                .unwrap();
            exs.push(ex);
        }

        let cache = HashMap::from([(tr.uuid(), (tr.clone(), stub_trigger_payload()))]);

        eq.init_queue_state_from_cache(cache, &cfg, &mut c)
            .await
            .unwrap();

        (eq, c, tr, exs.try_into().unwrap())
    }

    #[test_case(Status::Initialising, true, false, false, Some(EventData::ResolveConfig); "initialising")]
    #[test_case(Status::Resolving, true, false, false, Some(EventData::ResolveConfig); "resolving")]
    #[test_case(Status::Provisioning, true, true, true, Some(EventData::CreateEnvArgoWorkflow); "provisioning")]
    #[test_case(Status::EnvironmentReady, true, true, true, Some(EventData::CreateScenarioJob); "environment ready")]
    #[test_case(Status::Running, true, true, false, Some(EventData::WaitForScenarioJob); "running")]
    #[test_case(Status::Successful, false, false, false, None; "successful")]
    #[test_case(Status::Failed, false, false, false, None; "failed")]
    #[test_case(Status::Unrunnable, false, false, false, None; "unrunnable")]
    #[tokio::test]
    async fn init_queue_state_produces_expected_state(
        status: Status,
        registered: bool,
        running: bool,
        cached: bool,
        data: Option<EventData>,
    ) {
        let (mut eq, _, tr, [ex]) = prepare_init_queue_test([status]).await;
        let ex_uuid = ex.uuid();
        let run_uuid = tr.uuid();

        eq.with_shared(|shared| {
            if registered {
                assert_eq!(
                    shared.executions.get(&ex_uuid).map(|e| e.run_uuid),
                    Some(run_uuid),
                    "ex should be in executions"
                );

                assert_eq!(
                    shared.runs.get(&run_uuid).map(|run| run.executions.clone()),
                    Some(HashSet::from([ex_uuid])),
                    "ex should be in run's executions"
                );
            }

            assert_eq!(
                shared
                    .executions
                    .get(&ex_uuid)
                    .is_some_and(|e| e.resolved_config.is_some()),
                cached,
                "incorrect cached execution config state"
            );
        })
        .await;

        eq.with_inner(|inner| {
            assert_eq!(
                inner
                    .running_executions()
                    .get(&alpha_cluster())
                    .is_some_and(|r| r.contains(&ex_uuid)),
                running,
                "incorrect running state"
            );
        })
        .await;

        match (eq.rx.try_recv(), data) {
            (Ok(evt), Some(EventData::ResolveConfig)) => assert_eq!(
                evt,
                QueueEvent::Provision(DEFAULT_POOL, PendingProvision::new(ex, run_uuid, false,))
            ),

            (Ok(evt), Some(data)) => assert_eq!(
                evt,
                QueueEvent::Other(Event::new(ex, alpha_cluster(), data))
            ),

            (Err(_), None) => (),

            (Ok(evt), None) => panic!("unexpected event: {evt:?}"),
            (Err(_), Some(data)) => panic!("expected {data:?} but got None"),
        }
    }

    #[test_case(Status::Running, Status::Running, true; "both in flight")]
    #[test_case(Status::Running, Status::Successful, true; "one in flight")]
    #[test_case(Status::Failed, Status::Successful, false; "neither in flight")]
    #[tokio::test]
    async fn init_queue_state_evicts_the_payload_cache_correctly(
        s1: Status,
        s2: Status,
        still_cached: bool,
    ) {
        let (eq, conn, tr, _) = prepare_init_queue_test([s1, s2]).await;

        eq.with_shared(|shared| {
            assert_eq!(
                shared.runs.contains_key(&tr.uuid()),
                still_cached,
                "incorrect in-memory cache state"
            );
        })
        .await;

        assert_eq!(
            conn.cleared_payload_caches.contains(&tr.uuid()),
            !still_cached,
            "incorrect DB cache state"
        );
    }

    #[test_case(EventData::ResolveConfig, false; "unprovisioned execution")]
    #[test_case(EventData::WaitForScenarioJob, true; "running execution")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn purge_execution_clears_queue_state(
        data: EventData,
        is_running: bool,
    ) -> db::Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &DEFAULT_POOL, c).await?;
        let ex = tr.init_execution("test", 0, c).await?;
        let ex_id = ex.uuid();
        let tr_id = tr.uuid();

        let cfg = Config::for_test();
        let (mut eq, ph, eqs, _) = EventQueue::new(&cfg.workload_clusters);
        cache_stub_run(&ph, tr.uuid()).await;
        eq.with_shared(|s| s.register_execution(ex_id, tr_id)).await;
        if is_running {
            eq.with_inner(|i| i.insert_running_execution(ex_id, alpha_cluster()))
                .await;
        }
        let evt = match data {
            EventData::ResolveConfig => provision(DEFAULT_POOL, ex.clone()),
            data => QueueEvent::Other(Event {
                test_execution: ex.clone(),
                cluster: alpha_cluster(),
                data,
            }),
        };
        eq.push_event(evt).await;

        eqs.purge_execution(ex.clone(), "X".into(), c).await;

        eq.with_inner(|inner| {
            // We need to match like this as the different cases result in these fields being
            // either Some(empty_collection) or None depending on the parameters
            match inner.pending_provisions().get(&DEFAULT_POOL) {
                Some(v) if !v.is_empty() => panic!("expected empty queue, got {v:?}"),
                _ => (),
            }
            match inner.running_executions().get(&alpha_cluster()) {
                Some(s) if !s.is_empty() => panic!("expected nothing running, got {s:?}"),
                _ => (),
            }

            if is_running {
                // should have queued the namespace purge
                assert_eq!(inner.pending_non_provisions().len(), 1);
                let front = inner.pending_non_provisions().front().unwrap();
                assert_matches!(
                    front,
                    Event {
                        test_execution: ex,
                        data: EventData::PurgeNamespace,
                        ..
                    } if ex.uuid() == ex_id,
                );
            } else {
                assert!(inner.pending_non_provisions().is_empty());
            }
        })
        .await;

        // should have nothing cached
        eqs.with_shared(|shared| {
            assert!(shared.runs.is_empty(), "run should be evicted");
            assert!(shared.executions.is_empty(), "executions should be empty");
        })
        .await;

        // status should be correct
        let status = ex.try_current_status(c).await?.unwrap();
        assert_eq!(status.status, Status::Cancelled);
        assert_eq!(status.message.as_deref(), Some("cancelled by X"));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn purge_run_cancels_and_purges_non_terminal_executions() -> db::Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &DEFAULT_POOL, c).await?;
        let ex_1 = tr.init_execution("queued", 0, c).await?;
        let ex_2 = tr.init_execution("running", 1, c).await?;
        let ex_3 = tr.init_execution("done", 2, c).await?;
        ex_3.set_status(Status::Successful, None, c).await?;

        let cfg = Config::for_test();
        let (mut eq, ph, eqs, _) = EventQueue::new(&cfg.workload_clusters);
        cache_stub_run(&ph, tr.uuid()).await;

        // 1 & 2 are ongoing
        eq.with_shared(|shared| {
            shared.register_execution(ex_1.uuid(), tr.uuid());
            shared.register_execution(ex_2.uuid(), tr.uuid());
        })
        .await;

        // 1 is provisioning
        eq.push_event(provision(DEFAULT_POOL, ex_1.clone())).await;

        // 2 is running
        eq.with_inner(|i| i.insert_running_execution(ex_2.uuid(), alpha_cluster()))
            .await;

        eqs.purge_run(tr.clone(), "X".into(), c).await;

        // should have nothing cached
        eqs.with_shared(|shared| {
            assert!(shared.runs.is_empty(), "run should be evicted");
            assert!(shared.executions.is_empty(), "executions should be empty");
        })
        .await;

        // queue should be empty
        eq.with_inner(|inner| {
            let running = inner.running_executions().get(&alpha_cluster()).unwrap();
            assert!(running.is_empty(), "running_executions should be empty");
            let pending = inner.pending_provisions().get(&DEFAULT_POOL).unwrap();
            assert!(pending.is_empty(), "pending_provisions should be empty");
        })
        .await;

        // statuses should be correct
        let run_status = tr.try_current_status(c).await?.unwrap();
        assert_eq!(run_status.status, Status::Cancelled);

        for (ex, status) in [
            (ex_1, Status::Cancelled),
            (ex_2, Status::Cancelled),
            (ex_3, Status::Successful),
        ] {
            assert_eq!(ex.try_current_status(c).await?.unwrap().status, status);
        }

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn request_provisioning_sees_purged_runs() -> db::Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &DEFAULT_POOL, c).await?;

        let cfg = Config::for_test();
        let (_eq, ph, eqs, _) = EventQueue::new(&cfg.workload_clusters);
        cache_stub_run(&ph, tr.uuid()).await;

        eqs.purge_run(tr.clone(), "X".into(), c).await;

        let mut mock = MockUpdateHandle::with_run(tr.clone());
        let res = ph
            .request_provisioning(&tr, "test", 0, DEFAULT_POOL, &mut mock)
            .await;

        assert_matches!(res, Ok(false), "should have reported cancellation");
        assert!(
            mock.test_executions.is_empty(),
            "should have nothing queued"
        );

        Ok(())
    }

    fn cluster_a() -> ClusterId {
        ClusterId::new("a")
    }

    fn cluster_b() -> ClusterId {
        ClusterId::new("b")
    }

    fn two_cluster_pool(max_concurrent: usize, dedicated: bool) -> WorkloadClusters {
        let mut clusters =
            WorkloadClusters::for_test_with_available_clusters(max_concurrent, "a", &["a", "b"]);
        clusters.cluster_pools.default.available_clusters = vec!["a".into(), "b".into()];
        clusters.cluster_pools.default.supports_dedicated = dedicated;
        clusters.cluster_pools.additional.clear();

        clusters
    }

    async fn snapshot_state(
        clusters: &WorkloadClusters,
        initiator: &str,
    ) -> (EventQueue, ProvisioningHandle, EventQueueState, Uuid) {
        let (eq, ph, state, _) = EventQueue::new(clusters);
        let run_uuid = Uuid::new_v4();
        let ctx = OrchestratorContext::new_from_inlined_files(
            &Config::for_test(),
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        ph.cache_for_test_run(
            run_uuid,
            Some(initiator.to_string()),
            Default::default(),
            true,
            ctx,
            stub_test_plan(),
        )
        .await;

        (eq, ph, state, run_uuid)
    }

    #[tokio::test]
    async fn snapshot_reports_pending_provisions_against_their_pool() {
        let (mut eq, _, state, run_uuid) = snapshot_state(&two_cluster_pool(10, true), "a").await;
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        eq.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
            .await;
        eq.push_event(QueueEvent::Provision(
            DEFAULT_POOL.clone(),
            PendingProvision::new(ex.clone(), run_uuid, true),
        ))
        .await;

        let snapshot = state.event_queue_snapshot().await;

        assert_eq!(
            snapshot.pools["default"].pending_provisions,
            vec![PendingProvisionSummary {
                execution_id: ex.uuid(),
                run_id: run_uuid,
                requires_dedicated: true,
            }]
        );
        assert_eq!(
            snapshot.clusters.keys().collect::<Vec<_>>(),
            vec!["a", "b"],
            "pools must not appear in the cluster map"
        );
    }

    #[tokio::test]
    async fn snapshot_reports_owned_and_reserved_claims() {
        let (eq, _ph, state, owner) = snapshot_state(&two_cluster_pool(10, true), "alice").await;
        let reserver = Uuid::new_v4();
        eq.with_inner(|inner| {
            inner
                .claims_mut()
                .insert(cluster_a(), ClusterClaim::Owned(owner));
            inner
                .claims_mut()
                .insert(cluster_b(), ClusterClaim::Reserved(reserver));
            inner.insert_running_execution(Uuid::new_v4(), cluster_b());
            inner.insert_running_execution(Uuid::new_v4(), cluster_b());
        })
        .await;

        let snapshot = state.event_queue_snapshot().await;

        assert_eq!(
            snapshot.clusters["a"].claim,
            Some(ClusterClaimSummary::Owned {
                run_id: owner,
                initiated_by: Some("alice".to_string()),
            })
        );
        assert_eq!(
            snapshot.clusters["b"].claim,
            Some(ClusterClaimSummary::Reserved {
                run_id: reserver,
                initiated_by: None,
                executions_to_wait_for: 2,
            })
        );
    }

    async fn prepare_dedicated_recovery(
        dedicated_runs: &[i32],
        runs: &[(TestRun, &[Status])],
    ) -> (EventQueue, MockUpdateHandle, Vec<TestExecution>) {
        let cfg = Config::for_test();
        let (mut q, _, _, _) = EventQueue::new(&cfg.workload_clusters);

        let mut c = MockUpdateHandle {
            test_runs: runs.iter().map(|(tr, _)| tr.clone()).collect(),
            dedicated_runs: dedicated_runs.to_vec(),
            ..Default::default()
        };

        let mut exs = Vec::new();
        let mut cache = HashMap::new();
        for (tr, statuses) in runs.iter() {
            for status in statuses.iter() {
                let ex = c.init_execution(tr, "test", 0).await.unwrap();
                _ = c.update_test_execution_status(&ex, *status, None).await;
                exs.push(ex);
            }
            cache.insert(tr.uuid(), (tr.clone(), stub_trigger_payload()));
        }

        _ = q.init_queue_state_from_cache(cache, &cfg, &mut c).await;

        (q, c, exs)
    }

    #[tokio::test]
    async fn init_queue_state_restores_dedicated_ownership() {
        let tr = TestRun::create_stub(1, "test");
        let (q, _, _) = prepare_dedicated_recovery(
            &[tr.id()],
            &[(tr.clone(), &[Status::Running, Status::Initialising])],
        )
        .await;

        assert_eq!(
            q.inner.lock().await.cluster_claim_for(&alpha_cluster()),
            Some(ClusterClaim::Owned(tr.uuid()))
        );
    }

    #[tokio::test]
    async fn init_queue_state_purges_runs_sharing_a_dedicated_cluster() {
        let dedicated = TestRun::create_stub(1, "dedicated");
        let other = TestRun::create_stub(2, "other");
        let (q, mut c, exs) = prepare_dedicated_recovery(
            &[dedicated.id()],
            &[
                (dedicated.clone(), &[Status::Running]),
                (other.clone(), &[Status::Running]),
            ],
        )
        .await;

        for ex in exs.iter() {
            let status = c
                .try_current_test_execution_status(ex)
                .await
                .unwrap()
                .map(|s| s.status);
            assert_eq!(
                status,
                Some(Status::Unrunnable),
                "execution {} should be unrunnable",
                ex.id()
            );
        }

        q.with_inner(|inner| {
            assert!(inner.running_executions().values().all(|r| r.is_empty()));

            let purges = inner
                .pending_non_provisions()
                .iter()
                .filter(|evt| evt.data == EventData::PurgeNamespace)
                .count();
            assert_eq!(purges, 2, "both namespaces should be purged");
        })
        .await;
        q.with_shared(|shared| {
            assert!(shared.runs.is_empty(), "runs should be evicted");
            assert!(shared.executions.is_empty(), "executions should be evicted");
        })
        .await;
    }
}
