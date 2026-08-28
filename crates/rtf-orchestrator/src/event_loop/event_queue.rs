use crate::{
    config::{Config, WorkloadClusters},
    context::OrchestratorContext,
    db::{self, ClusterId, Status, StatusTracked, TestExecution, TestRun, UpdateHandle},
    event_loop::{Event, EventData},
    resolver::{self, ResolverError, ResolverInput},
    state::TestRunWithPayload,
};
use rtf_config::{
    StableSource,
    checks::Check,
    context::ResolutionContext,
    inlining::InlineMode,
    run::RunProviders,
    templating::{Template, TemplateContext},
};
use rtf_orchestrator_shared::{
    OutputCollectionResponse, PrometheusQueries,
    payload::PreparedPayload,
    test_plan::{OrchestratorEnvironment, OrchestratorTestPlan},
};
use serde::Serialize;
use sqlx::PgConnection;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};
use tokio::sync::{
    Mutex,
    mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel},
};
use tracing::{error, warn};
use uuid::Uuid;

/// Coordinates queuing of k8s events to provide back pressure and prioritise running executions
/// over newly submitted ones.
///
/// Held by the event loop task with paired [ProvisioningHandle] and [EventQueueState] structs that
/// are used elsewhere in the codebase to submit events to the queue and introspect the current
/// queue state.
#[derive(Debug)]
pub struct EventQueue {
    /// Sender for submitting events back to the queue
    tx: UnboundedSender<Event>,
    /// Receiver for accepting new events
    rx: UnboundedReceiver<Event>,
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
            inner: Arc::new(Mutex::new(EventQueueInner::new(
                cfg.available_clusters(),
                cfg.max_concurrent_executions(),
            ))),
            tx_resolve: tx_resolve.clone(),
        };

        let ph = ProvisioningHandle {
            shared: eq.shared.clone(),
            tx: eq.tx.clone(),
        };

        let eqs = EventQueueState {
            shared: eq.shared.clone(),
            eq_inner: Arc::clone(&eq.inner),
            tx_resolve,
            default_cluster: cfg.default_workload_cluster(),
        };

        (eq, ph, eqs, rx_resolve)
    }

    pub fn tx(&self) -> UnboundedSender<Event> {
        self.tx.clone()
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
    async fn push_event(&mut self, evt: Event) {
        self.with_inner(|inner| match &evt.data {
            EventData::ResolveConfig => inner
                .pending_provisions
                .entry(evt.cluster.clone())
                .or_default()
                .push_back(evt),
            _ => inner.pending_non_provisions.push_back(evt),
        })
        .await
    }

    fn still_have_external_senders(&self) -> bool {
        self.rx.sender_strong_count() > 1
    }

    /// Returns the next [Event] to be processed, prioritising non-provision events over
    /// provisioning new namespaces.
    ///
    /// We buffer events internally and fully drain the channel of any events received since the
    /// last call to `next_event`. This method blocks when there are no internally buffered events
    /// and the channel is currently empty or if we are running `max_concurrent_executions` and the
    /// only queued events would provision a new namespace.
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

    pub(crate) async fn scenario_docker_image_and_command(
        &self,
        ex_id: Uuid,
    ) -> Option<(String, String)> {
        self.with_shared(|shared| {
            shared
                .executions
                .get(&ex_id)
                .and_then(|e| e.resolved_config.as_ref())
                .map(|c| (c.docker_image.clone(), c.docker_command.clone()))
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
        let h = ProvisioningHandle {
            shared: self.shared.clone(),
            tx: self.tx.clone(),
        };

        for (run_uuid, (tr, payload)) in cache.into_iter() {
            let PreparedPayload {
                test_plan,
                relative_files,
                custom_providers,
                ..
            } = payload;
            let ctx =
                OrchestratorContext::new_from_inlined_files(cfg, relative_files, custom_providers);
            h.cache_for_test_run(
                run_uuid,
                tr.initiated_by().map(|s| s.to_owned()),
                ctx,
                test_plan,
            )
            .await;

            let executions = conn.executions_for_run(&tr).await?;
            let mut n_in_flight = 0;
            let cluster = tr.workload_cluster();

            for ex in executions.into_iter() {
                let events = self
                    .try_recover_execution(ex, run_uuid, cluster.clone(), &h, conn)
                    .await?;
                for evt in events.into_iter() {
                    let _ = self.tx.send(evt);
                    n_in_flight += 1;
                }
            }

            if n_in_flight == 0 {
                // Nothing left to do for this run so evict from the cache
                h.evict_cached_run_state(run_uuid).await;
                conn.clear_cached_payload_for_run(run_uuid).await;
            }
        }

        Ok(())
    }

    async fn try_recover_execution(
        &mut self,
        ex: TestExecution,
        run_uuid: Uuid,
        cluster: ClusterId,
        h: &ProvisioningHandle,
        conn: &mut impl UpdateHandle,
    ) -> crate::Result<Vec<Event>> {
        let current = match conn.try_current_test_execution_status(&ex).await? {
            Some(s) if s.status.is_terminal() => return Ok(Vec::new()),
            None => Status::Initialising,
            Some(s) => s.status,
        };

        let ex_uuid = ex.uuid();

        self.with_shared(|shared| shared.register_execution(ex_uuid, run_uuid))
            .await;

        if current > Status::Resolving {
            self.with_inner(|inner| inner.insert_running_execution(ex_uuid, cluster.clone()))
                .await;
        }

        let data = match current {
            Status::Successful | Status::Failed | Status::Unrunnable => {
                unreachable!("is_terminal() checked above")
            }

            // No side-effecting actions taken yet so we're clear to run the full event flow.
            Status::Initialising | Status::Resolving => vec![EventData::ResolveConfig],

            // This execution would have previously claimed a running execution slot but we don't
            // know how far through the provisioning process we were. So we mark it as running to
            // reserve the slot and then run the full event flow.
            // This will resolve the config before attempting to create the workflow which is
            // idempotent, allowing us to skip straight to waiting for the workflow to complete if
            // needed.
            Status::Provisioning => match h.resolve_and_cache_config(&ex).await {
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
            Status::EnvironmentReady => match h.resolve_and_cache_config(&ex).await {
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

        Ok(data
            .into_iter()
            .map(|data| Event {
                test_execution: ex.clone(),
                cluster: cluster.clone(),
                data,
            })
            .collect())
    }
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
    /// Pending provisioning events for new executions, queued per cluster
    pending_provisions: HashMap<ClusterId, VecDeque<Event>>,
    /// Ordering for obtaining the next queueable provisioning event, visited round-robin
    cluster_provision_order: VecDeque<ClusterId>,
    /// Uuids for the set of executions whose namespaces are live, per cluster
    running_executions: HashMap<ClusterId, HashSet<Uuid>>,
    /// Maximum number of live namespaces, per cluster
    max_concurrent_executions: HashMap<ClusterId, usize>,
}

impl EventQueueInner {
    fn new(
        available_clusters: Vec<ClusterId>,
        max_concurrent_executions: HashMap<ClusterId, usize>,
    ) -> Self {
        Self {
            pending_non_provisions: VecDeque::new(),
            pending_provisions: HashMap::new(),
            cluster_provision_order: available_clusters.into(),
            running_executions: HashMap::new(),
            max_concurrent_executions,
        }
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

    /// Find the next cluster in round-robin order with both a queued provisioning event and
    /// free namespace capacity, and pop its front event.
    fn runnable_provisioning_event(&mut self) -> Option<Event> {
        for _ in 0..self.cluster_provision_order.len() {
            let cluster = self.cluster_provision_order.front()?.clone();
            self.cluster_provision_order.rotate_left(1);

            let running = self.running_executions.get(&cluster).map_or(0, |s| s.len());
            let max = self
                .max_concurrent_executions
                .get(&cluster)
                .copied()
                .unwrap_or(0);

            if running >= max {
                continue;
            }

            if let Some(evt) = self
                .pending_provisions
                .get_mut(&cluster)
                .and_then(|q| q.pop_front())
            {
                return Some(evt);
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

/// A handle for submitting provisioning requests to the [EventQueue].
#[derive(Debug, Clone)]
pub struct ProvisioningHandle {
    /// Shared state with the parent event queue
    shared: Arc<Mutex<Shared>>,
    /// Sender for submitting provisioning events to the event queue
    tx: UnboundedSender<Event>,
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

    /// Decrement `n_queued` and submit a provisioning event to the event loop.
    ///
    /// The event waits in `EventQueue::pending_provisions` until namespace capacity is
    /// available — the gating happens inside `EventQueue::next_event`, not here.
    pub(crate) async fn request_provisioning(
        &self,
        ex: TestExecution,
        run_uuid: Uuid,
        cluster: ClusterId,
    ) -> resolver::Result<()> {
        self.with_shared(|shared| {
            assert!(
                shared.n_queued > 0,
                "request_provisioning called with n_queued == 0"
            );

            shared.n_queued -= 1;
            shared.register_execution(ex.uuid(), run_uuid);
        })
        .await;

        if let Err(e) = self.tx.send(Event {
            test_execution: ex,
            cluster,
            data: EventData::ResolveConfig,
        }) {
            error!(%e, "event loop channel closed");
            return Err(ResolverError::EventChannelClosed);
        }

        Ok(())
    }

    async fn templated_and_checked_version(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<(OrchestratorTestPlan, Arc<OrchestratorContext>)> {
        let (mut test_plan, ctx) = self
            .with_shared(|shared| shared.variant_with_context(ex))
            .await?;

        let template_ctx = TemplateContext::new(
            test_plan.variables.clone(),
            HashMap::new(),
            ctx.custom_provider_definitions(),
        );

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
            .inline(
                &InlineMode::All,
                ctx.as_ref(),
                &mut *inline_cache.lock().await,
            )
            .await?;

        test_plan
            .scenario
            .execution
            .inline(
                &InlineMode::All,
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
            .send(Event {
                test_execution: ex,
                cluster,
                data,
            })
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
    default_cluster: ClusterId,
}

impl EventQueueState {
    async fn with_shared<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Shared) -> T,
    {
        f(&mut *self.shared.lock().await)
    }

    pub fn default_cluster(&self) -> &ClusterId {
        &self.default_cluster
    }

    pub(crate) async fn initiator_for_run(&self, run_uuid: Uuid) -> Option<String> {
        self.with_shared(|shared| shared.runs.get(&run_uuid)?.initiated_by.clone())
            .await
    }

    /// Count how many of `user`'s runs and executions on `cluster` are currently queued (waiting
    /// for a namespace) versus concurrent (holding one).
    ///
    /// A run counts as concurrent as soon as any one of its executions holds a namespace slot on
    /// `cluster`, mirroring the high-water-mark semantics already used for run status. It counts
    /// as queued only while none of its executions have reached that point yet. Executions are
    /// counted individually, with no such deduplication by run.
    pub async fn user_queue_counts(&self, user: &str, cluster: &ClusterId) -> UserQueueCounts {
        let (queued, running) = {
            let inner = self.eq_inner.lock().await;
            (
                inner
                    .pending_provisions
                    .get(cluster)
                    .map(|q| q.iter().map(|evt| evt.test_execution.uuid()).collect())
                    .unwrap_or_else(Vec::new),
                inner
                    .running_executions
                    .get(cluster)
                    .cloned()
                    .unwrap_or_default(),
            )
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

    pub async fn available_clusters(&self) -> Vec<ClusterId> {
        let mut cluster_ids: Vec<_> = self
            .eq_inner
            .lock()
            .await
            .cluster_provision_order
            .iter()
            .cloned()
            .collect();
        cluster_ids.sort_unstable();

        cluster_ids
    }

    pub async fn event_queue_snapshot(&self) -> Snapshot {
        let (pending_non_provisions, pending_provisions, running_executions) = {
            let inner = self.eq_inner.lock().await;
            let pending_non_provisions: Vec<_> = inner
                .pending_non_provisions
                .iter()
                .map(|evt| EventSummary {
                    execution_id: evt.test_execution.uuid(),
                    cluster: evt.cluster.clone(),
                    data: evt.data.clone(),
                })
                .collect();

            let pending_provisions: Vec<_> = inner
                .pending_provisions
                .values()
                .flatten()
                .map(|evt| EventSummary {
                    execution_id: evt.test_execution.uuid(),
                    cluster: evt.cluster.clone(),
                    data: evt.data.clone(),
                })
                .collect();

            let running_executions: Vec<_> = inner
                .running_executions
                .values()
                .flatten()
                .cloned()
                .collect();

            (
                pending_non_provisions,
                pending_provisions,
                running_executions,
            )
        };

        self.with_shared(|shared| {
            let summary = SnapshotSummary {
                running: running_executions.len(),
                queued: shared.n_queued,
                pending_provisions: pending_provisions.len(),
                pending_non_provisions: pending_non_provisions.len(),
            };

            Snapshot {
                summary,
                pending_non_provisions,
                pending_provisions,
                running_executions,
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
        test_run: TestRun,
        payload: PreparedPayload,
    ) -> Result<(), SubmitError> {
        let n = payload.test_plan.matrix.n_variants();
        if claim.0 != n {
            return Err(SubmitError::InvalidClaim);
        }

        match self
            .tx_resolve
            .send(ResolverInput::TestRun(Box::new(TestRunWithPayload {
                test_run,
                payload,
            }))) {
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
}

#[derive(Debug)]
struct RunState {
    ctx: Arc<OrchestratorContext>,
    test_plan: OrchestratorTestPlan,
    executions: HashSet<Uuid>,
    initiated_by: Option<String>,
}

#[derive(Debug)]
struct ExecutionState {
    run_uuid: Uuid,
    resolved_config: Option<ResolvedExecutionConfig>,
}

/// All config resolved for a single execution ahead of time, kept together since it's always
/// inserted and evicted as a unit.
#[derive(Debug, Clone)]
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

#[derive(Debug, Serialize)]
pub struct Snapshot {
    summary: SnapshotSummary,
    pending_non_provisions: Vec<EventSummary>,
    pending_provisions: Vec<EventSummary>,
    running_executions: Vec<Uuid>,
    cached_run_payloads: Vec<Uuid>,
    active_run_executions: HashMap<Uuid, HashSet<Uuid>>,
    resolved_execution_cache: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct EventSummary {
    execution_id: Uuid,
    cluster: ClusterId,
    data: EventData,
}

#[derive(Debug, Serialize)]
pub struct SnapshotSummary {
    running: usize,
    queued: usize,
    pending_provisions: usize,
    pending_non_provisions: usize,
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
        config::Config,
        context::OrchestratorContext,
        db::{MockUpdateHandle, Queryable},
        event_loop::tests::stub_test_plan,
    };
    use rtf_config::{formats::NullEnvironment, templating::Scalar};
    use rtf_orchestrator_shared::payload::SourceKeyedArrayMap;
    use simple_test_case::test_case;
    use std::{collections::HashMap, time::Duration};

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

    async fn populated_prov_handle() -> (ProvisioningHandle, TestExecution) {
        let cfg = Config::for_test();
        let (_, ph, _, _) = EventQueue::new(&cfg.workload_clusters);
        let run_uuid = Uuid::new_v4();
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
        );
        ph.cache_for_test_run(run_uuid, None, ctx, stub_test_plan())
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

    #[tokio::test]
    async fn initiated_by_for_run_returns_the_recorded_initiator() {
        let cfg = Config::for_test();
        let (_, ph, state, _) = EventQueue::new(&cfg.workload_clusters);
        let run_uuid = Uuid::new_v4();

        let ctx = OrchestratorContext::new_from_inlined_files(
            &cfg,
            empty_source_map(),
            empty_source_map(),
        );
        ph.cache_for_test_run(
            run_uuid,
            Some("alice@example.com".to_string()),
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

    fn resolve_config_event(ex: &TestExecution, cluster: ClusterId) -> Event {
        Event {
            test_execution: ex.clone(),
            cluster,
            data: EventData::ResolveConfig,
        }
    }

    #[test_case(&[], UserQueueCounts::new(0, 0, 0); "nothing queued at all")]
    #[test_case(&[("alice", "a", &[false])], UserQueueCounts::new(0, 1, 1); "single execution")]
    #[test_case(&[("alice", "a", &[true])], UserQueueCounts::new(1, 0, 0); "single ongoing run")]
    #[test_case(&[("alice", "a", &[false, false])], UserQueueCounts::new(0, 1, 2); "one run with two queued executions")]
    #[test_case(&[("alice", "a", &[true, false])], UserQueueCounts::new(1, 0, 1); "single ongoing execution marks run ongoing")]
    #[test_case(&[("alice", "b", &[false])], UserQueueCounts::new(0, 0, 0); "runs for another cluster are ignored")]
    #[test_case(&[("alice", "a", &[false]), ("bob", "a", &[false])], UserQueueCounts::new(0, 1, 1); "runs for another user are ignored")]
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
        );

        let mut ex_id = 1;
        for (user, cluster, statuses) in queued_executions.iter() {
            let run_uuid = Uuid::new_v4();
            let cluster = ClusterId::new(*cluster);

            ph.cache_for_test_run(
                run_uuid,
                Some(user.to_string()),
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
                    eq.push_event(resolve_config_event(&ex, cluster.clone()))
                        .await
                }
            }

            ex_id += statuses.len() as i32;
        }

        assert_eq!(
            state.user_queue_counts("alice", &ClusterId::new("a")).await,
            expected
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
        );
        h.cache_for_test_run(run_uuid, None, ctx, stub_test_plan())
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

    #[tokio::test]
    async fn request_provisioning_happy_path() {
        let (mut q, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let ex_uuid = ex.uuid();
        q.shared.lock().await.n_queued = 1;

        let res = h
            .request_provisioning(ex, Uuid::new_v4(), alpha_cluster())
            .await;

        assert!(res.is_ok(), "should have been able to send event: {res:?}");

        let shared = q.shared.lock().await;
        assert_eq!(shared.n_queued, 0, "n_queued should have been decremented");
        drop(shared);

        let event = q.rx.try_recv().expect("event should have been sent");

        assert_eq!(event.test_execution.uuid(), ex_uuid);
        assert!(matches!(event.data, EventData::ResolveConfig));
    }

    #[tokio::test]
    #[should_panic(expected = "request_provisioning called with n_queued == 0")]
    async fn request_provisioning_panics_when_n_queued_is_zero() {
        let (_, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        _ = h
            .request_provisioning(ex, Uuid::new_v4(), alpha_cluster())
            .await;
    }

    #[tokio::test]
    async fn request_provisioning_returns_false_when_channel_is_closed() {
        let (q, h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        q.shared.lock().await.n_queued = 1;
        drop(q);

        let res = h
            .request_provisioning(
                TestExecution::create_stub(1, 1, 0, "test"),
                Uuid::new_v4(),
                alpha_cluster(),
            )
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
        q.tx.send(Event {
            test_execution: ex,
            cluster: alpha_cluster(),
            data: EventData::ResolveConfig,
        })
        .unwrap();

        let res = tokio::time::timeout(Duration::from_millis(50), q.next_event()).await;
        assert!(res.is_err(), "next_event should have blocked at capacity");

        let _ = q.mark_execution_complete(blocking_id).await;
        let evt = tokio::time::timeout(Duration::from_secs(1), q.next_event())
            .await
            .expect("next_event should have yielded within 1s")
            .expect("should have returned Some(event)");

        assert_eq!(evt.test_execution.uuid(), ex_uuid);
        assert!(matches!(evt.data, EventData::ResolveConfig));

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
    async fn next_event_round_robins_provisions_across_clusters() {
        let (mut q, _h, _, _) =
            EventQueue::new(&WorkloadClusters::for_test_with_available_clusters(
                10,
                "alpha",
                &["alpha", "router_perf"],
            ));

        // Both of alpha's events are pushed before either of router_perf's, so a FIFO-only queue
        // would dispatch them back-to-back; round-robin fairness should still alternate clusters.
        for (cluster, ex_id) in [
            (alpha_cluster(), 1),
            (alpha_cluster(), 2),
            (perf_cluster(), 3),
            (perf_cluster(), 4),
        ] {
            q.push_event(Event {
                test_execution: TestExecution::create_stub(ex_id, 1, 0, "test"),
                cluster,
                data: EventData::ResolveConfig,
            })
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
            "expected clusters to alternate rather than draining alpha before router_perf"
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

        q.push_event(Event {
            test_execution: alpha_ex,
            cluster: alpha_cluster(),
            data: EventData::ResolveConfig,
        })
        .await;
        q.push_event(Event {
            test_execution: perf_ex,
            cluster: perf_cluster(),
            data: EventData::ResolveConfig,
        })
        .await;

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

    fn provision_evt(ex_id: i32) -> Event {
        Event {
            test_execution: TestExecution::create_stub(ex_id, 1, 0, "test"),
            cluster: alpha_cluster(),
            data: EventData::ResolveConfig,
        }
    }

    fn cleanup_evt(ex_id: i32) -> Event {
        Event {
            test_execution: TestExecution::create_stub(ex_id, 1, 0, "test"),
            cluster: alpha_cluster(),
            data: EventData::CleanupNamespace,
        }
    }

    #[test_case(vec![provision_evt(1), provision_evt(2)], 1; "only provision")]
    #[test_case(vec![cleanup_evt(1), cleanup_evt(2)], 1; "only non-provision")]
    #[test_case(vec![provision_evt(1), cleanup_evt(2)], 2; "both")]
    #[tokio::test]
    async fn next_event_returns_expected_event_from_pending(events: Vec<Event>, expected_id: i32) {
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
    async fn push_event_routes_resolve_env_config_to_provisions() {
        let (mut q, _h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let evt = Event {
            test_execution: TestExecution::create_stub(1, 1, 0, "test"),
            cluster: alpha_cluster(),
            data: EventData::ResolveConfig,
        };

        q.push_event(evt).await;

        q.with_inner(|inner| {
            assert_eq!(
                inner
                    .pending_provisions
                    .get(&alpha_cluster())
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(inner.pending_non_provisions.len(), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn push_event_routes_create_env_argo_workflow_to_non_provisions() {
        let (mut q, _h, _, _) = EventQueue::new(&WorkloadClusters::for_test());
        let evt = Event {
            test_execution: TestExecution::create_stub(1, 1, 0, "test"),
            cluster: alpha_cluster(),
            data: EventData::CreateEnvArgoWorkflow,
        };

        q.push_event(evt).await;

        q.with_inner(|inner| {
            assert!(!inner.pending_provisions.contains_key(&alpha_cluster()));
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
            assert!(
                matches!(
                    cached.environment,
                    OrchestratorEnvironment::DockerCompose(_)
                ),
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
        );
        ph.cache_for_test_run(run_uuid, None, ctx, test_plan).await;
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

            assert!(
                matches!(cached.environment, OrchestratorEnvironment::Null(_)),
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
        assert!(
            matches!(res, Some(OrchestratorEnvironment::Null(_))),
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

        assert!(
            matches!(res, Err(ResolverError::UnknownExecution(_))),
            "expected UnknownExecution, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn resolve_scenario_for_execution_returns_error_when_cache_empty() {
        let (_, _, eqs, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eqs.resolve_scenario_for_execution(&ex).await;

        assert!(
            matches!(res, Err(ResolverError::UnknownExecution(_))),
            "expected UnknownExecution, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn resolve_output_collection_for_execution_returns_error_when_cache_empty() {
        let (_, _, eqs, _) = EventQueue::new(&WorkloadClusters::for_test());
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eqs.resolve_output_collection_for_execution(&ex).await;

        assert!(
            matches!(res, Err(ResolverError::UnknownExecution(_))),
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
        assert!(
            matches!(input, ResolverInput::ResolveConfig(ref e, _) if e.uuid() == ex.uuid()),
            "wrong input forwarded: {input:?}"
        );
    }

    fn stub_trigger_payload() -> PreparedPayload {
        PreparedPayload {
            variables: None,
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
            (Ok(evt), Some(data)) => assert_eq!(
                evt,
                Event {
                    test_execution: ex,
                    cluster: alpha_cluster(),
                    data
                }
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
}
