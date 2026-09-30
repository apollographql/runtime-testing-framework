use crate::{
    Error,
    config::{Config, WorkloadClusters},
    context::OrchestratorContext,
    db::{self, ClusterId, PoolId, Status, StatusTracked, TestExecution, TestRun, UpdateHandle},
    event_loop::{Event, EventData, PendingProvision, QueueEvent},
    resolver::{self, ResolverError, ResolverInput},
    state::TestRunWithPayload,
};
use rtf_config::{
    StableSource,
    checks::Check,
    inlining::{Inline, InlineMode},
    templating::Template,
};
use rtf_orchestrator_shared::{
    OutputCollectionResponse, PrometheusQueries,
    event_queue::{ClusterQueueState, EventQueueSnapshot, EventSummary, SnapshotSummary},
    payload::PreparedPayload,
    test_plan::{OrchestratorEnvironment, OrchestratorTestPlan},
};
use sqlx::PgConnection;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
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

        let eqs = EventQueueState {
            shared: eq.shared.clone(),
            eq_inner: Arc::clone(&eq.inner),
            tx_resolve,
            pool_clusters: cfg.pool_clusters(),
            dedicated_pools: cfg.dedicated_pools(),
        };

        (eq, ph, eqs, rx_resolve)
    }

    pub fn tx(&self) -> UnboundedSender<QueueEvent> {
        self.tx.clone()
    }

    fn provisioning_handle(&self) -> ProvisioningHandle {
        ProvisioningHandle {
            shared: self.shared.clone(),
            tx: self.tx.clone(),
        }
    }

    async fn with_shared<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Shared) -> T,
    {
        f(&mut *self.shared.lock().await)
    }

    async fn with_inner<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut EventQueueInner) -> T,
    {
        f(&mut *self.inner.lock().await)
    }

    pub async fn is_empty(&self) -> bool {
        self.rx.is_empty()
            && self
                .with_inner(|inner| {
                    inner.pending_provisions.values().all(|q| q.is_empty())
                        && inner.pending_non_provisions.is_empty()
                })
                .await
    }

    #[inline(always)]
    async fn push_event(&mut self, evt: QueueEvent) {
        self.with_inner(|inner| match evt {
            QueueEvent::Provision(pool, evt) => inner
                .pending_provisions
                .entry(pool)
                .or_default()
                .push_back(evt),
            QueueEvent::Other(evt) => inner.pending_non_provisions.push_back(evt),
        })
        .await
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

    /// Immediately remove any outstanding reservations and schedule a ReleaseCluster event for any
    /// clusters currently claimed by this run.
    pub async fn schedule_cluster_release(&mut self, run_uuid: Uuid, ex: &TestExecution) {
        self.with_inner(|inner| inner.schedule_cluster_release(run_uuid, ex))
            .await
    }

    /// Remove any claims currently in place for the given cluster ID
    pub async fn clear_cluster_claims_for(&mut self, cluster: &ClusterId) {
        self.with_inner(|inner| {
            inner.claims.remove(cluster);
        })
        .await
    }

    pub(crate) async fn scenario_job_params(&self, ex_id: Uuid) -> Option<ScenarioJobParams> {
        self.with_shared(|shared| {
            let ex_state = shared.executions.get(&ex_id)?;
            let ex_cfg = ex_state.resolved_config.as_ref()?;
            let allow_k8s_write = shared.runs.get(&ex_state.run_uuid)?.allow_k8s_write;

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
                .with_inner(|inner| {
                    inner
                        .pool_clusters
                        .get(&pool)
                        .and_then(|c| c.first().cloned())
                })
                .await
                .ok_or_else(|| Error::UnknownWorkloadPool {
                    pool: pool.to_string(),
                })?;

            let requires_dedicated = conn.run_requires_dedicated_cluster(&tr).await?;
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
                tr.allow_k8s_write(),
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
        for (cluster, claim) in inner.claims.iter() {
            if !matches!(claim, ClusterClaim::Owned(_)) {
                continue;
            }

            let runs: HashSet<Uuid> = inner
                .running_executions
                .get(cluster)
                .into_iter()
                .flatten()
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
                    inner
                        .claims
                        .entry(cluster.clone())
                        .or_insert(ClusterClaim::Owned(run_uuid));
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

#[derive(Debug)]
struct EventQueueInner {
    /// Pending events for in-progress executions, independent of cluster
    pending_non_provisions: VecDeque<Event>,
    /// Pending provisioning events for new executions, queued per pool
    pending_provisions: HashMap<PoolId, VecDeque<PendingProvision>>,
    /// Ordering for obtaining the next queueable provisioning event, visited round-robin
    pool_provision_order: VecDeque<PoolId>,
    /// The clusters that make up each pool
    pool_clusters: HashMap<PoolId, Vec<ClusterId>>,
    /// Pools that runs can claim dedicated clusters from
    dedicated_pools: HashSet<PoolId>,
    /// Uuids for the set of executions whose namespaces are live, per cluster
    running_executions: HashMap<ClusterId, HashSet<Uuid>>,
    /// Maximum number of live namespaces, per cluster
    max_concurrent_executions: HashMap<ClusterId, usize>,
    /// Clusters that are reserved for, or owned by, a run that requires a dedicated cluster
    claims: HashMap<ClusterId, ClusterClaim>,
}

impl EventQueueInner {
    fn new(cfg: &WorkloadClusters) -> Self {
        Self {
            pending_non_provisions: VecDeque::new(),
            pending_provisions: HashMap::new(),
            pool_provision_order: cfg.available_pools().into(),
            pool_clusters: cfg.pool_clusters(),
            dedicated_pools: cfg.dedicated_pools(),
            running_executions: HashMap::new(),
            max_concurrent_executions: cfg.max_concurrent_executions(),
            claims: HashMap::new(),
        }
    }

    fn running_on(&self, cluster: &ClusterId) -> usize {
        self.running_executions.get(cluster).map_or(0, |s| s.len())
    }

    fn has_capacity(&self, cluster: &ClusterId) -> bool {
        let max = self
            .max_concurrent_executions
            .get(cluster)
            .copied()
            .unwrap_or(0);

        self.running_on(cluster) < max
    }

    /// Immediately remove any outstanding reservations and schedule a ReleaseCluster event for any
    /// clusters currently claimed by this run.
    fn schedule_cluster_release(&mut self, run_uuid: Uuid, ex: &TestExecution) {
        let mut released = Vec::new();

        self.claims.retain(|cluster, claim| match claim {
            ClusterClaim::Reserved(run) if *run == run_uuid => false,
            ClusterClaim::Owned(run) if *run == run_uuid => {
                released.push(cluster.clone());
                true
            }
            _ => true,
        });

        for cluster in released.into_iter() {
            self.pending_non_provisions.push_front(Event::new(
                ex.clone(),
                cluster,
                EventData::ReleaseCluster,
            ));
        }
    }

    /// Attempt to find a cluster that is able to accepts a new execution from the given test run.
    fn try_assign(
        &mut self,
        pool: &PoolId,
        run_uuid: Uuid,
        requires_dedicated: bool,
    ) -> Option<(ClusterId, EventData)> {
        use ClusterClaim::{Owned, Reserved};

        let clusters = self.pool_clusters.get(pool)?;
        let cluster_with_claim = |claim| {
            clusters
                .iter()
                .find(|c| self.claims.get(*c) == Some(&claim))
        };

        // If we don't need a dedicated cluster and the pool doesn't support dedicated,
        // assign to the least loaded cluster (provided we have capacity)
        if !(requires_dedicated && self.dedicated_pools.contains(pool)) {
            let cluster = clusters
                .iter()
                .filter(|c| !self.claims.contains_key(*c) && self.has_capacity(c))
                .min_by_key(|c| self.running_on(c))?;

            return Some((cluster.clone(), EventData::ResolveConfig));
        }

        // All assignments past this point are for runs requiring dedicated clusters

        // If this run already owns a cluster, assign to that cluster (provided we have capacity)
        if let Some(cluster) = cluster_with_claim(Owned(run_uuid)) {
            return self
                .has_capacity(cluster)
                .then(|| (cluster.clone(), EventData::ResolveConfig));
        }

        // If this run has an outstanding reservation assign to that cluster if it is now free and
        // update the claim to Owned (otherwise we continue waiting)
        if let Some(cluster) = cluster_with_claim(Reserved(run_uuid)) {
            if self.running_on(cluster) > 0 || !self.has_capacity(cluster) {
                return None;
            }
            self.claims.insert(cluster.clone(), Owned(run_uuid));
            return Some((cluster.clone(), EventData::AcquireCluster));
        }

        let unclaimed: Vec<&ClusterId> = clusters
            .iter()
            .filter(|c| !self.claims.contains_key(*c))
            .collect();

        // If we have an empty unclaimed cluster, claim it and assign to it
        let first_empty = unclaimed.iter().find(|c| self.running_on(c) == 0);
        if let Some(cluster) = first_empty {
            self.claims.insert((*cluster).clone(), Owned(run_uuid));

            return Some(((*cluster).clone(), EventData::AcquireCluster));
        }

        // Finally, if we have at least one unclaimed cluster then reserve the one with the fewest
        // ongoing executions before returning None
        if let Some(cluster) = unclaimed.iter().min_by_key(|c| self.running_on(c)) {
            self.claims.insert((*cluster).clone(), Reserved(run_uuid));
        }

        None
    }

    fn insert_running_execution(&mut self, ex_uuid: Uuid, cluster: ClusterId) {
        self.running_executions
            .entry(cluster.clone())
            .or_default()
            .insert(ex_uuid);
    }

    fn remove_running_execution(&mut self, ex_uuid: Uuid) -> bool {
        for running in self.running_executions.values_mut() {
            if running.remove(&ex_uuid) {
                return true;
            }
        }

        false
    }

    fn cluster_for_execution(&mut self, ex_uuid: Uuid) -> Option<ClusterId> {
        for (cid, running) in self.running_executions.iter() {
            if running.contains(&ex_uuid) {
                return Some(cid.clone());
            }
        }

        None
    }

    /// Find the next pool in round-robin order with a queued provisioning event that can be
    /// assigned to a cluster, and remove that event from the queue.
    ///
    /// Events within a pool are considered in order, but one that is waiting on a dedicated
    /// cluster does not block the events behind it.
    fn runnable_provisioning_event(&mut self) -> Option<Event> {
        for _ in 0..self.pool_provision_order.len() {
            let pool = self.pool_provision_order.front()?.clone();
            self.pool_provision_order.rotate_left(1);

            let n_pending = self.pending_provisions.get(&pool).map_or(0, VecDeque::len);
            for i in 0..n_pending {
                let (run_uuid, requires_dedicated) = {
                    let pending = &self.pending_provisions[&pool][i];
                    (pending.run_uuid, pending.requires_dedicated)
                };

                let (cluster, data) = match self.try_assign(&pool, run_uuid, requires_dedicated) {
                    Some(details) => details,
                    None => continue,
                };

                let provision = self.pending_provisions.get_mut(&pool)?.remove(i)?;

                return Some(Event::new(provision.test_execution, cluster, data));
            }
        }

        None
    }

    fn next_event(&mut self) -> Option<Event> {
        if let Some(evt) = self.pending_non_provisions.pop_front() {
            return Some(evt);
        } else if let Some(evt) = self.runnable_provisioning_event() {
            self.insert_running_execution(evt.test_execution.uuid(), evt.cluster.clone());
            return Some(evt);
        }

        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClusterClaim {
    Reserved(Uuid),
    Owned(Uuid),
}

/// A handle for submitting provisioning requests to the [EventQueue].
#[derive(Debug, Clone)]
pub struct ProvisioningHandle {
    /// Shared state with the parent event queue
    shared: Arc<Mutex<Shared>>,
    /// Sender for submitting provisioning events to the event queue
    tx: UnboundedSender<QueueEvent>,
}

impl ProvisioningHandle {
    /// Run a closure with access to the shared event queue state.
    ///
    /// This method holds the mutex lock on the shared state for the duration of the closure. As
    /// such, you _must_ ensure that closures are quick to execute. If multiple long running
    /// operations are required, prefer extracting the state you need from [Shared] and
    /// manipulating it after releasing the lock before re-acquiring.
    async fn with_shared<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Shared) -> T,
    {
        f(&mut *self.shared.lock().await)
    }

    pub(crate) async fn cache_for_test_run(
        &self,
        run_uuid: Uuid,
        initiated_by: Option<String>,
        allow_k8s_write: bool,
        requires_dedicated: bool,
        ctx: OrchestratorContext,
        test_plan: OrchestratorTestPlan,
    ) {
        self.with_shared(|shared| {
            shared.runs.insert(
                run_uuid,
                RunState {
                    ctx: Arc::new(ctx),
                    test_plan,
                    executions: HashSet::new(),
                    initiated_by,
                    allow_k8s_write,
                    requires_dedicated,
                },
            )
        })
        .await;
    }

    /// Drop the in-memory run entry for the given run uuid without touching the ref-count
    /// bookkeeping. Used by the resolver when it has cached a payload but every `init_execution`
    /// for the run failed — there will be no `mark_execution_complete` for this run, so eviction
    /// has to happen here instead.
    pub(crate) async fn evict_cached_run_state(&self, run_uuid: Uuid) {
        self.with_shared(|shared| {
            shared.runs.remove(&run_uuid);
        })
        .await;
    }

    /// Create the DB row for a new execution and submit a provisioning event to the event loop,
    /// decrementing `n_queued` in the process.
    ///
    /// The event waits in `EventQueue::pending_provisions` until namespace capacity is
    /// available — the gating happens inside `EventQueue::next_event`, not here.
    pub(crate) async fn request_provisioning<H>(
        &self,
        tr: &TestRun,
        name: &str,
        index: usize,
        pool: PoolId,
        update_handle: &mut H,
    ) -> crate::Result<bool>
    where
        H: UpdateHandle,
    {
        let mut shared = self.shared.lock().await;

        let requires_dedicated = match shared.runs.get(&tr.uuid()) {
            Some(r) => r.requires_dedicated,
            None => {
                // run was cancelled while we were resolving executions
                return Ok(false);
            }
        };

        let ex = update_handle.init_execution(tr, name, index).await?;

        assert!(
            shared.n_queued > 0,
            "request_provisioning called with n_queued == 0"
        );
        shared.n_queued -= 1;
        shared.register_execution(ex.uuid(), tr.uuid());
        drop(shared);

        if let Err(e) = self.tx.send(QueueEvent::Provision(
            pool,
            PendingProvision::new(ex, tr.uuid(), requires_dedicated),
        )) {
            error!(%e, "event loop channel closed");
            return Err(ResolverError::EventChannelClosed.into());
        }

        Ok(true)
    }

    /// Register an already-created execution against its parent run's in-memory state, without
    /// creating a DB row or submitting a provisioning event.
    #[cfg(test)]
    pub(crate) async fn register_existing_execution(&self, ex_uuid: Uuid, run_uuid: Uuid) {
        self.with_shared(|shared| shared.register_execution(ex_uuid, run_uuid))
            .await;
    }

    async fn templated_and_checked_version(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<(OrchestratorTestPlan, Arc<OrchestratorContext>)> {
        let (mut test_plan, ctx) = self
            .with_shared(|shared| shared.variant_with_context(ex))
            .await?;

        let template_ctx = ctx.new_template_context(test_plan.variables.clone());

        test_plan
            .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
            .map_err(ResolverError::VariantTemplating)?;

        test_plan.try_check(&mut Vec::new(), ctx.as_ref())?;

        Ok((test_plan, ctx))
    }

    /// Resolve the environment and scenario config for `ex` and store it in
    /// `resolved_execution_cache` keyed by execution UUID.
    pub(crate) async fn resolve_and_cache_config(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<()> {
        let (mut test_plan, ctx) = self.templated_and_checked_version(ex).await?;

        // Make sure that we run the run inlining without holding the lock on shared
        let inline_cache = ctx.inline_cache();
        test_plan
            .environment
            .execution
            .try_inline(
                InlineMode::All,
                ctx.as_ref(),
                &mut *inline_cache.lock().await,
            )
            .await?;

        test_plan
            .scenario
            .execution
            .try_inline(
                InlineMode::All,
                ctx.as_ref(),
                &mut *inline_cache.lock().await,
            )
            .await?;

        // Clear custom provider details to allow `rtf resolve` to run
        test_plan.environment.custom_providers = Vec::new();
        test_plan.scenario.custom_providers = Vec::new();

        let image = test_plan.scenario.execution.docker_image();
        let command = test_plan.scenario.execution.command();
        let environment = test_plan.environment.execution.clone();

        let env_yaml = serde_yaml::to_string(&test_plan.environment)
            .map_err(|e| ResolverError::Serialisation(e.to_string()))?;
        let scenario_yaml = serde_yaml::to_string(&test_plan.scenario)
            .map_err(|e| ResolverError::Serialisation(e.to_string()))?;
        let execution_variables = serde_json::to_string_pretty(&test_plan.variables)
            .map_err(|e| ResolverError::Serialisation(e.to_string()))?;

        let env_prom_queries = test_plan
            .environment
            .execution
            .prometheus_queries()
            .iter()
            .map(|q| {
                q.with_namespace_label_filter(&ex.uuid().to_string())
                    .expect("promql query should have been validated by try_check")
            })
            .collect();
        let scenario_prom_queries = test_plan
            .scenario
            .execution
            .output_collection
            .prometheus
            .iter()
            .map(|q| {
                q.with_namespace_label_filter(&ex.uuid().to_string())
                    .expect("promql query should have been validated by try_check")
            })
            .collect();
        let output_collection = OutputCollectionResponse {
            execution_variables,
            prometheus: PrometheusQueries {
                environment: env_prom_queries,
                scenario: scenario_prom_queries,
            },
        };

        self.with_shared(|shared| {
            let exec = shared
                .executions
                .get_mut(&ex.uuid())
                .ok_or(ResolverError::UnknownExecution(ex.uuid()))?;

            exec.resolved_config = Some(ResolvedExecutionConfig {
                env_yaml,
                scenario_yaml,
                output_collection,
                docker_image: image,
                docker_command: command,
                environment,
            });

            Ok(())
        })
        .await
    }

    /// Send an event to the event loop.
    pub(crate) fn send_event(
        &self,
        ex: TestExecution,
        cluster: ClusterId,
        data: EventData,
    ) -> resolver::Result<()> {
        self.tx
            .send(QueueEvent::Other(Event::new(ex, cluster, data)))
            .map_err(|_| ResolverError::EventChannelClosed)
    }

    /// Return `n` queued execution claims back to the shared state.
    ///
    /// # Safety
    /// The caller must pass an `n` that matches the number of unused claims they previously
    /// obtained from `try_reserve_pending_executions`.
    pub async unsafe fn release_pending_execution_claim(&self, n: usize) {
        let mut shared = self.shared.lock().await;
        shared.n_queued -= n;
    }
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

/// Access to shared [EventQueue] state.
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
    async fn with_shared<F, T>(&self, f: F) -> T
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
            let queued: Vec<Uuid> = inner
                .pending_provisions
                .get(pool)
                .into_iter()
                .flatten()
                .map(|evt| evt.test_execution.uuid())
                .collect();
            let running: HashSet<Uuid> = clusters
                .iter()
                .filter_map(|c| inner.running_executions.get(c))
                .flatten()
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

    pub fn supports_dedicated(&self, pool: &PoolId) -> bool {
        self.dedicated_pools.contains(pool)
    }

    pub fn available_pools(&self) -> Vec<PoolId> {
        let mut pool_ids: Vec<_> = self.pool_clusters.keys().cloned().collect();
        pool_ids.sort_unstable();

        pool_ids
    }

    pub async fn event_queue_snapshot(&self) -> EventQueueSnapshot {
        let (clusters, mut summary) = {
            let inner = self.eq_inner.lock().await;
            let mut clusters: BTreeMap<String, ClusterQueueState> = inner
                .pool_clusters
                .values()
                .flatten()
                .map(|cid| (cid.to_string(), ClusterQueueState::default()))
                .collect();
            let mut summary = SnapshotSummary::default();

            for (cid, running) in inner.running_executions.iter() {
                let state = clusters.entry(cid.to_string()).or_default();
                state.running_executions.extend(running.iter().copied());
                state.running_executions.sort_unstable();
                summary.running += running.len();
            }

            // Queued provisions have not been assigned a cluster yet so they are reported against
            // the name of their pool.
            for (pool, events) in inner.pending_provisions.iter() {
                let state = clusters.entry(pool.to_string()).or_default();
                for evt in events.iter() {
                    state.pending_provisions.push(EventSummary {
                        execution_id: evt.test_execution.uuid(),
                        event: EventData::ResolveConfig.name().to_string(),
                    });
                    summary.pending_provisions += 1;
                }
            }

            for evt in inner.pending_non_provisions.iter() {
                let state = clusters.entry(evt.cluster.to_string()).or_default();
                state.pending_non_provisions.push(event_summary(evt));
                summary.pending_non_provisions += 1;
            }

            (clusters, summary)
        };

        self.with_shared(|shared| {
            summary.queued = shared.n_queued;

            EventQueueSnapshot {
                summary,
                clusters,
                cached_run_payloads: shared.runs.keys().cloned().collect(),
                active_run_executions: shared
                    .runs
                    .iter()
                    .map(|(run_uuid, run)| (*run_uuid, run.executions.clone()))
                    .collect(),
                resolved_execution_cache: shared
                    .executions
                    .iter()
                    .filter(|(_, exec)| exec.resolved_config.is_some())
                    .map(|(ex_uuid, _)| *ex_uuid)
                    .collect(),
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

async fn purge_execution_inner(
    ex: TestExecution,
    shared: &mut Shared,
    inner: &mut EventQueueInner,
    conn: &mut impl UpdateHandle,
) -> Option<()> {
    let ex_id = ex.uuid();

    // drop provision events first to ensure that we don't create any _new_ namespaces
    for events in inner.pending_provisions.values_mut() {
        events.retain(|evt| evt.test_execution.uuid() != ex_id)
    }

    if let Some(cluster) = inner.cluster_for_execution(ex_id) {
        // The execution made it as far as being provisioned so we need to nuke the namespace
        inner.running_executions.get_mut(&cluster)?.remove(&ex_id);
        inner
            .pending_non_provisions
            .retain(|evt| evt.test_execution.uuid() != ex_id);
        inner.pending_non_provisions.push_front(Event::new(
            ex.clone(),
            cluster,
            EventData::PurgeNamespace,
        ));
    };

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

#[derive(Debug)]
struct RunState {
    ctx: Arc<OrchestratorContext>,
    test_plan: OrchestratorTestPlan,
    executions: HashSet<Uuid>,
    initiated_by: Option<String>,
    allow_k8s_write: bool,
    requires_dedicated: bool,
}

#[derive(Debug, PartialEq)]
struct ExecutionState {
    run_uuid: Uuid,
    resolved_config: Option<ResolvedExecutionConfig>,
}

/// All config resolved for a single execution ahead of time, kept together since it's always
/// inserted and evicted as a unit.
#[derive(Debug, Clone, PartialEq)]
struct ResolvedExecutionConfig {
    env_yaml: String,
    scenario_yaml: String,
    output_collection: OutputCollectionResponse,
    docker_image: String,
    docker_command: String,
    environment: OrchestratorEnvironment,
}

#[derive(Debug)]
struct Shared {
    /// Metadata for every run with at least one in-flight execution, keyed by TestRun uuid.
    runs: HashMap<Uuid, RunState>,
    /// Metadata for every in-flight TestExecution, keyed by its uuid.
    executions: HashMap<Uuid, ExecutionState>,
    /// Maximum number of pending executions waiting for a namespace
    max_queued_executions: usize,
    /// The number of currently queued executions
    n_queued: usize,
}

impl Shared {
    fn register_execution(&mut self, ex_uuid: Uuid, run_uuid: Uuid) {
        self.executions.insert(
            ex_uuid,
            ExecutionState {
                run_uuid,
                resolved_config: None,
            },
        );

        if let Some(run) = self.runs.get_mut(&run_uuid) {
            run.executions.insert(ex_uuid);
        }
    }

    fn variant_with_context(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<(OrchestratorTestPlan, Arc<OrchestratorContext>)> {
        let run_uuid = match self.executions.get(&ex.uuid()) {
            Some(exec) => exec.run_uuid,
            None => return Err(ResolverError::UnknownExecution(ex.uuid())),
        };
        let index = ex.test_plan_index();

        match self.runs.get(&run_uuid) {
            Some(run) => match run.test_plan.try_expand_variant(index)? {
                Some((_, variant)) => Ok((variant, Arc::clone(&run.ctx))),
                None => Err(ResolverError::UnknownExecution(ex.uuid())),
            },
            None => Err(ResolverError::UnknownRun(run_uuid)),
        }
    }
}

fn event_summary(evt: &Event) -> EventSummary {
    EventSummary {
        execution_id: evt.test_execution.uuid(),
        event: evt.data.name().to_string(),
    }
}

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
    use super::*;
    use crate::{
        config::{Config, DEFAULT_POOL},
        conn,
        context::OrchestratorContext,
        db::{MockUpdateHandle, Queryable},
        event_loop::tests::stub_test_plan,
    };
    use rtf_config::{formats::NullEnvironment, templating::Scalar};
    use rtf_orchestrator_shared::payload::SourceKeyedArrayMap;
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

    async fn populated_prov_handle() -> (ProvisioningHandle, TestExecution) {
        let cfg = Config::for_test();
        let (_, ph, _, _) = EventQueue::new(&cfg.workload_clusters);
        let run_uuid = Uuid::new_v4();
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
            HashMap::new(),
        );
        ph.cache_for_test_run(run_uuid, None, false, false, ctx, stub_test_plan())
            .await;
        ph.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
            .await;

        (ph, ex)
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
    async fn mark_execution_complete_updates_running_set() {
        let (mut eq, _, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let id = Uuid::new_v4();
        eq.with_inner(|inner| inner.insert_running_execution(id, alpha_cluster()))
            .await;

        let evicted = eq.mark_execution_complete(id).await;

        assert!(
            !eq.with_inner(|inner| inner
                .running_executions
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

    impl EventQueueState {
        async fn initiator_for_run(&self, run_uuid: Uuid) -> Option<String> {
            self.with_shared(|shared| shared.runs.get(&run_uuid)?.initiated_by.clone())
                .await
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
            false,
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
        let (mut eq, ph, state, _) = EventQueue::new(
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
                false,
                false,
                ctx.clone(),
                stub_test_plan(),
            )
            .await;

            for (i, &is_running) in statuses.iter().enumerate() {
                let ex = TestExecution::create_stub(ex_id + i as i32, 1, i, "test");
                ph.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
                    .await;

                if is_running {
                    eq.with_inner(|inner| {
                        inner.insert_running_execution(ex.uuid(), cluster.clone())
                    })
                    .await
                } else {
                    eq.push_event(QueueEvent::Provision(
                        pool.clone(),
                        PendingProvision::new(ex, run_uuid, false),
                    ))
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

        let (eq, ph, state, _) = EventQueue::new(&clusters);
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
            false,
            false,
            ctx,
            stub_test_plan(),
        )
        .await;

        for (i, cluster) in ["a", "b"].into_iter().enumerate() {
            let ex = TestExecution::create_stub(i as i32 + 1, 1, i, "test");
            ph.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
                .await;
            eq.with_inner(|inner| {
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
        h.cache_for_test_run(run_uuid, None, false, false, ctx, stub_test_plan())
            .await;
        h.with_shared(|shared| {
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
        h.cache_for_test_run(run_uuid, None, false, false, ctx, stub_test_plan())
            .await;
    }

    #[tokio::test]
    async fn request_provisioning_happy_path() {
        let (mut q, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let tr = TestRun::create_stub(1, "test");
        let mut mock = MockUpdateHandle::with_run(tr.clone());
        cache_stub_run(&h, tr.uuid()).await;
        q.shared.lock().await.n_queued = 1;

        let res = h
            .request_provisioning(&tr, "test", 0, DEFAULT_POOL, &mut mock)
            .await;

        assert_matches!(
            res,
            Ok(true),
            "should have submitted the execution: {res:?}"
        );

        let shared = q.shared.lock().await;
        assert_eq!(shared.n_queued, 0, "n_queued should have been decremented");
        drop(shared);

        let event = q.rx.try_recv().expect("event should have been sent");
        let ex_uuid = mock.test_executions[0].uuid();

        assert_matches!(
            event,
            QueueEvent::Provision(pool, PendingProvision { test_execution, run_uuid, requires_dedicated: false })
                if pool == DEFAULT_POOL
                    && test_execution.uuid() == ex_uuid
                    && run_uuid == tr.uuid()
        );
    }

    #[tokio::test]
    #[should_panic(expected = "request_provisioning called with n_queued == 0")]
    async fn request_provisioning_panics_when_n_queued_is_zero() {
        let (_, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let tr = TestRun::create_stub(1, "test");
        let mut mock = MockUpdateHandle::with_run(tr.clone());
        cache_stub_run(&h, tr.uuid()).await;

        _ = h
            .request_provisioning(&tr, "test", 0, DEFAULT_POOL, &mut mock)
            .await;
    }

    #[tokio::test]
    async fn request_provisioning_returns_false_when_channel_is_closed() {
        let (q, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let tr = TestRun::create_stub(1, "test");
        let mut mock = MockUpdateHandle::with_run(tr.clone());
        cache_stub_run(&h, tr.uuid()).await;
        q.shared.lock().await.n_queued = 1;
        drop(q);

        let res = h
            .request_provisioning(&tr, "test", 0, DEFAULT_POOL, &mut mock)
            .await;

        assert!(res.is_err(), "should have failed to send event");
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
                    .running_executions
                    .get(&alpha_cluster())
                    .is_some_and(|running| running.contains(&ex_uuid)),
                "running executions: {:?}",
                inner.running_executions
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
            q.inner.lock().await.pending_non_provisions.len(),
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
                inner.pending_provisions.get(&DEFAULT_POOL).unwrap().len(),
                1
            );
            assert_eq!(inner.pending_non_provisions.len(), 0);
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
            assert!(!inner.pending_provisions.contains_key(&DEFAULT_POOL));
            assert_eq!(inner.pending_non_provisions.len(), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn resolve_and_cache_config_stores_yaml_and_docker_details() {
        let (ph, ex) = populated_prov_handle().await;
        let ex_uuid = ex.uuid();

        let res = ph.resolve_and_cache_config(&ex).await;

        assert!(res.is_ok(), "{res:?}");
        ph.with_shared(|shared| {
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
        let (_, ph, _, _) = EventQueue::new(&cfg.workload_clusters);
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
        ph.cache_for_test_run(run_uuid, None, false, false, ctx, test_plan)
            .await;
        ph.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
            .await;

        let res = ph.resolve_and_cache_config(&ex).await;
        assert!(res.is_ok(), "{res:?}");

        ph.with_shared(|shared| {
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
        let (eq, ph, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eq.resolved_environment_for_execution(ex.uuid()).await;
        assert!(
            res.is_none(),
            "expected no cached environment before resolution, got {res:?}"
        );

        ph.with_shared(|shared| {
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
        let (mut eq, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let run_uuid = Uuid::new_v4();
        let ex_id = Uuid::new_v4();

        h.with_shared(|shared| {
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

        h.with_shared(|shared| {
            assert!(
                !shared.executions.contains_key(&ex_id),
                "execution entry should be evicted"
            );
        })
        .await;
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
                    .running_executions
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
        ph.with_shared(|s| s.register_execution(ex_id, tr_id)).await;
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
            match inner.pending_provisions.get(&DEFAULT_POOL) {
                Some(v) if !v.is_empty() => panic!("expected empty queue, got {v:?}"),
                _ => (),
            }
            match inner.running_executions.get(&alpha_cluster()) {
                Some(s) if !s.is_empty() => panic!("expected nothing running, got {s:?}"),
                _ => (),
            }

            if is_running {
                // should have queued the namespace purge
                assert_eq!(inner.pending_non_provisions.len(), 1);
                let front = inner.pending_non_provisions.front().unwrap();
                assert_matches!(
                    front,
                    Event {
                        test_execution: ex,
                        data: EventData::PurgeNamespace,
                        ..
                    } if ex.uuid() == ex_id,
                );
            } else {
                assert!(inner.pending_non_provisions.is_empty());
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
        ph.with_shared(|shared| {
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
            let running = inner.running_executions.get(&alpha_cluster()).unwrap();
            assert!(running.is_empty(), "running_executions should be empty");
            let pending = inner.pending_provisions.get(&DEFAULT_POOL).unwrap();
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
        clusters.cluster_pools.default.dedicated = dedicated;
        clusters.cluster_pools.additional.clear();

        clusters
    }

    // Helpers for testing the try_assign method without having to run the full next_event logic
    impl EventQueueInner {
        fn try_assign_insert(&mut self) -> Option<(ClusterId, EventData)> {
            let (cluster, data) = self.try_assign(&DEFAULT_POOL, Uuid::new_v4(), false)?;
            self.insert_running_execution(Uuid::new_v4(), cluster.clone());

            Some((cluster, data))
        }

        fn try_assign_insert_dedicated(
            &mut self,
            run_uuid: Uuid,
        ) -> Option<(ClusterId, EventData)> {
            let (cluster, data) = self.try_assign(&DEFAULT_POOL, run_uuid, true)?;
            self.insert_running_execution(Uuid::new_v4(), cluster.clone());

            Some((cluster, data))
        }
    }

    fn acquire(cluster: &str) -> (ClusterId, EventData) {
        (ClusterId::new(cluster), EventData::AcquireCluster)
    }

    fn resolve(cluster: &str) -> (ClusterId, EventData) {
        (ClusterId::new(cluster), EventData::ResolveConfig)
    }

    #[tokio::test]
    async fn try_assign_balances_executions_across_the_clusters_in_a_pool() {
        let (q, _, _, _) = EventQueue::new(&two_cluster_pool(10, false));
        let mut inner = q.inner.lock().await;

        assert_eq!(inner.try_assign_insert(), Some(resolve("a")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("a")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
    }

    #[tokio::test]
    async fn try_assign_only_uses_clusters_with_capacity() {
        let (q, _, _, _) = EventQueue::new(&two_cluster_pool(1, false));
        q.with_inner(|inner| inner.insert_running_execution(Uuid::new_v4(), cluster_a()))
            .await;
        let mut inner = q.inner.lock().await;

        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
        assert_eq!(inner.try_assign_insert(), None);
    }

    #[tokio::test]
    async fn try_assign_assigns_a_dedicated_run_to_a_single_cluster() {
        let (q, _, _, _) = EventQueue::new(&two_cluster_pool(2, true));
        let mut inner = q.inner.lock().await;
        let run = Uuid::new_v4();

        assert_eq!(inner.try_assign_insert_dedicated(run), Some(acquire("a")));
        assert_eq!(inner.try_assign_insert_dedicated(run), Some(resolve("a")));
        assert_eq!(inner.try_assign_insert_dedicated(run), None);
    }

    #[tokio::test]
    async fn try_assign_respects_dedicated_clusters() {
        let (q, _, _, _) = EventQueue::new(&two_cluster_pool(10, true));
        let mut inner = q.inner.lock().await;
        let run = Uuid::new_v4();

        assert_eq!(inner.try_assign_insert_dedicated(run), Some(acquire("a")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
        assert_eq!(inner.try_assign_insert_dedicated(run), Some(resolve("a")));
    }

    #[tokio::test]
    async fn try_assign_respects_reservations() {
        let (q, _, _, _) = EventQueue::new(&two_cluster_pool(10, true));
        let ex = Uuid::new_v4();
        q.with_inner(|inner| {
            inner.insert_running_execution(ex, cluster_a());
            inner.insert_running_execution(Uuid::new_v4(), cluster_b());
            inner.insert_running_execution(Uuid::new_v4(), cluster_b());
        })
        .await;

        let mut inner = q.inner.lock().await;
        let run = Uuid::new_v4();

        // we should reserve the cluster with the fewest running executions
        assert_eq!(inner.try_assign_insert_dedicated(run), None);
        assert_eq!(
            inner.claims.get(&cluster_a()),
            Some(&ClusterClaim::Reserved(run))
        );

        // once the reserved cluster has drained we should be able to acquire
        inner.remove_running_execution(ex);
        assert_eq!(inner.try_assign_insert_dedicated(run), Some(acquire("a")));
    }

    #[tokio::test]
    async fn try_assign_reserves_multiple_clusters() {
        let (q, _, _, _) = EventQueue::new(&two_cluster_pool(10, true));
        q.with_inner(|inner| {
            inner.insert_running_execution(Uuid::new_v4(), cluster_a());
            inner.insert_running_execution(Uuid::new_v4(), cluster_b());
        })
        .await;

        let mut inner = q.inner.lock().await;
        for (run, cluster) in [(Uuid::new_v4(), cluster_a()), (Uuid::new_v4(), cluster_b())] {
            assert_eq!(inner.try_assign_insert_dedicated(run), None);
            assert_eq!(
                inner.claims.get(&cluster),
                Some(&ClusterClaim::Reserved(run))
            );
        }
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
            q.inner.lock().await.claims,
            HashMap::from([(alpha_cluster(), ClusterClaim::Owned(tr.uuid()))])
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
            assert!(inner.running_executions.values().all(|r| r.is_empty()));

            let purges = inner
                .pending_non_provisions
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
