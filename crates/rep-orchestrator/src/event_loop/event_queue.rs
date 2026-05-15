use crate::{
    context::RepContext,
    db::{TestExecution, TestRun},
    event_loop::{Event, EventData},
    resolver::{self, ResolverError, ResolverInput},
    state::TestRunWithPayload,
};
use rep_orchestrator_shared::{payload::TriggerPayload, test_plan::RepTestPlan};
use rtf_config::{
    StableSource,
    checks::Check,
    context::ResolutionContext,
    inlining::InlineMode,
    run::RunProviders,
    templating::{Template, TemplateContext},
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::take,
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
    /// Maximum number of live namespaces running test executions
    max_concurrent_executions: usize,
    /// Sender for forwarding resolve events to the resolver_task
    tx_resolve: UnboundedSender<ResolverInput>,
}

impl EventQueue {
    /// Construct a new [EventQueue] along with its paired [ProvisioningHandle] and [EventQueueState]
    /// structs.
    pub fn new(
        max_concurrent_executions: usize,
        max_queued_executions: usize,
    ) -> (
        Self,
        ProvisioningHandle,
        EventQueueState,
        UnboundedReceiver<ResolverInput>,
    ) {
        let shared = Arc::new(Mutex::new(Shared {
            execution_map: HashMap::new(),
            payload_cache: HashMap::new(),
            active_run_executions: HashMap::new(),
            resolved_env_cache: HashMap::new(),
            resolved_scenario_cache: HashMap::new(),
            scenario_docker_cache: HashMap::new(),
            max_queued_executions,
            n_queued: 0,
        }));
        let (tx_resolve, rx_resolve) = unbounded_channel();
        let (tx, rx) = unbounded_channel();

        let eq = EventQueue {
            tx,
            rx,
            shared,
            inner: Arc::new(Mutex::new(EventQueueInner {
                pending_provisions: VecDeque::new(),
                pending_non_provisions: VecDeque::new(),
                running_executions: HashSet::new(),
            })),
            max_concurrent_executions,
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
                    inner.pending_provisions.is_empty() && inner.pending_non_provisions.is_empty()
                })
                .await
    }

    #[inline(always)]
    async fn push_event(&mut self, evt: Event) {
        self.with_inner(|inner| match &evt.data {
            EventData::ResolveConfig => inner.pending_provisions.push_back(evt),
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

            if let Some(evt) = self
                .with_inner(|inner| inner.next_event(self.max_concurrent_executions))
                .await
            {
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
        if !self.inner.lock().await.running_executions.remove(&ex_id) {
            warn!(%ex_id, "mark_execution_complete called for unknown execution id");
        }

        self.with_shared(|shared| {
            let run_uuid = shared.execution_map.remove(&ex_id)?;
            let active = shared.active_run_executions.get_mut(&run_uuid)?;
            active.remove(&ex_id);
            shared.resolved_scenario_cache.remove(&ex_id);
            shared.scenario_docker_cache.remove(&ex_id);

            if active.is_empty() {
                shared.active_run_executions.remove(&run_uuid);
                shared.payload_cache.remove(&run_uuid);
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
        self.with_shared(|shared| shared.scenario_docker_cache.get(&ex_id).cloned())
            .await
    }

    /// Send a [ResolverInput] to the resolver_task channel.
    pub(crate) fn send_to_resolver(&self, input: ResolverInput) -> resolver::Result<()> {
        self.tx_resolve
            .send(input)
            .map_err(|_| ResolverError::EventChannelClosed)
    }

    /// Evict the resolved env config cache entry for the given execution.
    pub(crate) async fn evict_resolved_env_config(&self, ex_id: Uuid) {
        self.with_shared(|shared| shared.resolved_env_cache.remove(&ex_id))
            .await;
    }
}

#[derive(Debug)]
struct EventQueueInner {
    /// Pending events for in-progress executions
    pending_non_provisions: VecDeque<Event>,
    /// Pending provisioning events for new executions
    pending_provisions: VecDeque<Event>,
    /// Uuids for the set of executions whose namespaces are live in the cluster.
    running_executions: HashSet<Uuid>,
}

impl EventQueueInner {
    fn runnable_provisioning_event(&mut self, max_concurrent_executions: usize) -> Option<Event> {
        if self.running_executions.len() < max_concurrent_executions {
            self.pending_provisions.pop_front()
        } else {
            None
        }
    }

    fn next_event(&mut self, max_concurrent_executions: usize) -> Option<Event> {
        if let Some(evt) = self.pending_non_provisions.pop_front() {
            return Some(evt);
        } else if let Some(evt) = self.runnable_provisioning_event(max_concurrent_executions) {
            self.running_executions.insert(evt.test_execution.uuid());
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
        ctx: RepContext,
        test_plan: RepTestPlan,
    ) {
        self.with_shared(|shared| {
            shared
                .payload_cache
                .insert(run_uuid, (Arc::new(ctx), test_plan))
        })
        .await;
    }

    /// Drop the in-memory payload cache entry for the given run uuid without touching the
    /// ref-count bookkeeping. Used by the resolver when it has cached a payload but every
    /// `init_execution` for the run failed — there will be no `mark_execution_complete` for
    /// this run, so eviction has to happen here instead.
    pub(crate) async fn evict_payload_cache(&self, run_uuid: Uuid) {
        self.with_shared(|shared| {
            shared.payload_cache.remove(&run_uuid);
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
    ) -> resolver::Result<(RepTestPlan, Arc<RepContext>)> {
        let (mut test_plan, ctx) = self
            .with_shared(|shared| shared.variant_with_context(ex))
            .await?;

        let variables = take(&mut test_plan.variables);
        let template_ctx =
            TemplateContext::new(variables, HashMap::new(), ctx.custom_provider_definitions());

        test_plan
            .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
            .map_err(ResolverError::VariantTemplating)?;

        test_plan.try_check(&mut Vec::new(), ctx.as_ref())?;

        Ok((test_plan, ctx))
    }

    /// Resolve the environment config for `ex`, serialise to YAML, and store in
    /// `resolved_env_cache` keyed by execution UUID.
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

        let env_yaml = serde_yaml::to_string(&test_plan.environment)
            .map_err(|e| ResolverError::Serialisation(e.to_string()))?;
        let scenario_yaml = serde_yaml::to_string(&test_plan.scenario)
            .map_err(|e| ResolverError::Serialisation(e.to_string()))?;

        self.with_shared(|shared| {
            shared.resolved_env_cache.insert(ex.uuid(), env_yaml);
            shared
                .resolved_scenario_cache
                .insert(ex.uuid(), scenario_yaml);
            shared
                .scenario_docker_cache
                .insert(ex.uuid(), (image, command));

            Ok(())
        })
        .await
    }

    /// Send an event to the event loop.
    pub(crate) fn send_event(&self, ex: TestExecution, data: EventData) -> resolver::Result<()> {
        self.tx
            .send(Event {
                test_execution: ex,
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
}

impl EventQueueState {
    async fn with_shared<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Shared) -> T,
    {
        f(&mut *self.shared.lock().await)
    }

    pub async fn event_queue_snapshot(&self) -> Snapshot {
        let (pending_non_provisions, pending_provisions, running_executions) = {
            let inner = self.eq_inner.lock().await;
            let pending_non_provisions: Vec<_> = inner
                .pending_non_provisions
                .iter()
                .map(|evt| EventSummary {
                    execution_id: evt.test_execution.uuid(),
                    data: evt.data.clone(),
                })
                .collect();

            let pending_provisions: Vec<_> = inner
                .pending_provisions
                .iter()
                .map(|evt| EventSummary {
                    execution_id: evt.test_execution.uuid(),
                    data: evt.data.clone(),
                })
                .collect();

            let running_executions: Vec<_> = inner.running_executions.iter().cloned().collect();

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
                cached_run_payloads: shared.payload_cache.keys().cloned().collect(),
                active_run_executions: shared.active_run_executions.clone(),
                resolved_env_cache: shared.resolved_env_cache.keys().cloned().collect(),
                resolved_scenario_cache: shared.resolved_env_cache.keys().cloned().collect(),
                resolved_docker_cache: shared.resolved_env_cache.keys().cloned().collect(),
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
        payload: TriggerPayload,
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
    pub async fn try_reserve_pending_executions(&self, tp: &RepTestPlan) -> Option<Claim> {
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
                .resolved_env_cache
                .get(&ex.uuid())
                .cloned()
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
                .resolved_scenario_cache
                .get(&ex.uuid())
                .cloned()
                .ok_or(ResolverError::UnknownExecution(ex.uuid()))
        })
        .await
    }
}

#[derive(Debug)]
struct Shared {
    /// Map of TestExecution uuid to parent TestRun uuid
    execution_map: HashMap<Uuid, Uuid>,
    /// Map of TestRun uuid to payload data
    payload_cache: HashMap<Uuid, (Arc<RepContext>, RepTestPlan)>,
    /// Executions active for each run.
    active_run_executions: HashMap<Uuid, HashSet<Uuid>>,
    /// Pre-resolved env config YAML keyed by execution UUID
    resolved_env_cache: HashMap<Uuid, String>,
    /// Pre-resolved scenario config YAML keyed by execution UUID
    resolved_scenario_cache: HashMap<Uuid, String>,
    /// Pre-resolved docker image and command keyed by execution UUID
    scenario_docker_cache: HashMap<Uuid, (String, String)>,
    /// Maximum number of pending executions waiting for a namespace
    max_queued_executions: usize,
    /// The number of currently queued executions
    n_queued: usize,
}

impl Shared {
    fn register_execution(&mut self, ex_uuid: Uuid, run_uuid: Uuid) {
        self.execution_map.insert(ex_uuid, run_uuid);
        self.active_run_executions
            .entry(run_uuid)
            .or_default()
            .insert(ex_uuid);
    }

    fn variant_with_context(
        &self,
        ex: &TestExecution,
    ) -> resolver::Result<(RepTestPlan, Arc<RepContext>)> {
        let run_uuid = match self.execution_map.get(&ex.uuid()) {
            Some(id) => id,
            None => return Err(ResolverError::UnknownExecution(ex.uuid())),
        };
        let index = ex.test_plan_index();

        match self.payload_cache.get(run_uuid) {
            Some((ctx, test_plan)) => match test_plan.try_expand_variant(index)? {
                Some((_, variant)) => Ok((variant, Arc::clone(ctx))),
                None => Err(ResolverError::UnknownExecution(ex.uuid())),
            },
            None => Err(ResolverError::UnknownRun(*run_uuid)),
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
    resolved_env_cache: Vec<Uuid>,
    resolved_scenario_cache: Vec<Uuid>,
    resolved_docker_cache: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct EventSummary {
    execution_id: Uuid,
    data: EventData,
}

#[derive(Debug, Serialize)]
pub struct SnapshotSummary {
    running: usize,
    queued: usize,
    pending_provisions: usize,
    pending_non_provisions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config, context::RepContext, db::Queryable, event_loop::tests::stub_test_plan,
    };
    use rep_orchestrator_shared::payload::SourceKeyedArrayMap;
    use rtf_config::templating::Scalar;
    use simple_test_case::test_case;
    use std::{collections::HashMap, time::Duration};

    fn dummy_config() -> Config {
        Config {
            apollo_key: "dummy".to_string(),
            db_host: "localhost".to_string(),
            db_port: 5432,
            db_name: "test".to_string(),
            db_user: "test".to_string(),
            db_pass: Some("test".to_string()),
            github_app_id: 1,
            github_app_private_key_pem: "dummy".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8035,
            max_concurrent_executions: 10,
            max_queued_executions: 100,
            kubeconfig_path: "dummy".to_string(),
            kubeconfig_secret_name: "workload-kubeconfig".to_string(),
            workload_context: "dummy".to_string(),
            orchestrator_url: "http://localhost:8035".to_string(),
            toolbox_pull_policy: "IfNotPresent".to_string(),
            gcs_bucket: "test-bucket".to_string(),
            gcs_url_ttl_secs: 300,
            mock_internal_gcs_url: Some("http://mock-gcs-internal".to_string()),
            mock_public_gcs_url: Some("http://mock-gcs-public".to_string()),
            failed_execution_ttl_secs: 600,
        }
    }

    fn empty_source_map<T>() -> SourceKeyedArrayMap<T> {
        SourceKeyedArrayMap {
            keys: vec![],
            data: vec![],
        }
    }

    async fn populated_prov_handle() -> (ProvisioningHandle, TestExecution) {
        let cfg = dummy_config();
        let (_, ph, _, _) = EventQueue::new(1, 100);
        let run_uuid = Uuid::new_v4();
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let ctx = RepContext::new(&cfg, empty_source_map(), empty_source_map());
        ph.cache_for_test_run(run_uuid, ctx, stub_test_plan()).await;
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
        let (_, _, state, _) = EventQueue::new(1, 5);

        for (i, &(n, expected)) in claims.iter().enumerate() {
            let mut tp = stub_test_plan();
            tp.matrix.dimensions = HashMap::from([("a".to_string(), vec![Scalar::Bool(true); n])]);

            let successful = state.try_reserve_pending_executions(&tp).await;
            assert_eq!(successful.is_some(), expected, "claim {i}");
        }
    }

    #[tokio::test]
    async fn mark_execution_complete_updates_running_set() {
        let (mut eq, _, _, _) = EventQueue::new(1, 5);
        let id = Uuid::new_v4();
        eq.inner.lock().await.running_executions.insert(id);

        let evicted = eq.mark_execution_complete(id).await;

        assert!(
            !eq.inner.lock().await.running_executions.contains(&id),
            "execution was still present in running set"
        );
        assert_eq!(
            evicted, None,
            "no run should be evicted when execution wasn't registered"
        );
    }

    #[tokio::test]
    async fn mark_execution_complete_evicts_cache_after_last_execution() {
        let (mut eq, h, _, _) = EventQueue::new(2, 5);
        let run_uuid = Uuid::new_v4();
        let ex1 = Uuid::new_v4();
        let ex2 = Uuid::new_v4();

        h.with_shared(|shared| {
            shared.register_execution(ex1, run_uuid);
            shared.register_execution(ex2, run_uuid);
        })
        .await;
        eq.with_inner(|inner| {
            inner.running_executions.insert(ex1);
            inner.running_executions.insert(ex2);
        })
        .await;

        // Completing the first execution leaves the run with 1 open execution: no eviction.
        let evicted = eq.mark_execution_complete(ex1).await;
        assert_eq!(evicted, None);
        eq.with_shared(|shared| {
            assert_eq!(
                shared
                    .active_run_executions
                    .get(&run_uuid)
                    .map(|set| set.len()),
                Some(1),
                "{:?}",
                shared.active_run_executions
            );
            assert!(shared.execution_map.contains_key(&ex2));
        })
        .await;

        let evicted = eq.mark_execution_complete(ex2).await;
        assert_eq!(evicted, Some(run_uuid),);

        eq.with_shared(|shared| {
            assert!(
                !shared.active_run_executions.contains_key(&run_uuid),
                "open count entry should be cleared"
            );
            assert!(
                !shared.execution_map.contains_key(&ex2),
                "ex1 should be removed"
            );
            assert!(
                !shared.execution_map.contains_key(&ex1),
                "ex2 should be removed"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn request_provisioning_happy_path() {
        let (mut q, h, _, _) = EventQueue::new(1, 5);
        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let ex_uuid = ex.uuid();
        q.shared.lock().await.n_queued = 1;

        let res = h.request_provisioning(ex, Uuid::new_v4()).await;

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
        let (_, h, _, _) = EventQueue::new(1, 5);
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        _ = h.request_provisioning(ex, Uuid::new_v4()).await;
    }

    #[tokio::test]
    async fn request_provisioning_returns_false_when_channel_is_closed() {
        let (q, h, _, _) = EventQueue::new(1, 5);
        q.shared.lock().await.n_queued = 1;
        drop(q);

        let res = h
            .request_provisioning(TestExecution::create_stub(1, 1, 0, "test"), Uuid::new_v4())
            .await;

        assert!(res.is_err(), "should have failed to send event");
    }

    #[tokio::test]
    async fn next_event_gates_provisions_on_namespace_capacity() {
        // max concurrent of 1: any provision dispatch is blocked while a slot is in use.
        let (mut q, _h, _, _) = EventQueue::new(1, 5);
        let blocking_id = Uuid::new_v4();
        q.inner.lock().await.running_executions.insert(blocking_id);

        let ex = TestExecution::create_stub(1, 1, 0, "test");
        let ex_uuid = ex.uuid();
        q.tx.send(Event {
            test_execution: ex,
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
                inner.running_executions.contains(&ex_uuid),
                "running executions: {:?}",
                inner.running_executions
            )
        })
        .await;
    }

    fn provision_evt(ex_id: i32) -> Event {
        Event {
            test_execution: TestExecution::create_stub(ex_id, 1, 0, "test"),
            data: EventData::ResolveConfig,
        }
    }

    fn cleanup_evt(ex_id: i32) -> Event {
        Event {
            test_execution: TestExecution::create_stub(ex_id, 1, 0, "test"),
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
        let (mut q, _h, _, _) = EventQueue::new(1, 5);

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
        let (mut q, _h, _, _) = EventQueue::new(1, 5);

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
        let (mut q, _h, _, _) = EventQueue::new(1, 5);
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
        let (mut q, h, _, _) = EventQueue::new(1, 5);

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
        let (mut q, _h, _, _) = EventQueue::new(1, 5);
        let evt = Event {
            test_execution: TestExecution::create_stub(1, 1, 0, "test"),
            data: EventData::ResolveConfig,
        };

        q.push_event(evt).await;

        q.with_inner(|inner| {
            assert_eq!(inner.pending_provisions.len(), 1);
            assert_eq!(inner.pending_non_provisions.len(), 0);
        })
        .await;
    }

    #[tokio::test]
    async fn push_event_routes_create_env_argo_workflow_to_non_provisions() {
        let (mut q, _h, _, _) = EventQueue::new(1, 5);
        let evt = Event {
            test_execution: TestExecution::create_stub(1, 1, 0, "test"),
            data: EventData::CreateEnvArgoWorkflow,
        };

        q.push_event(evt).await;

        q.with_inner(|inner| {
            assert_eq!(inner.pending_provisions.len(), 0);
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
            assert!(
                shared.resolved_env_cache.contains_key(&ex_uuid),
                "env YAML should be cached"
            );

            assert!(
                shared.resolved_scenario_cache.contains_key(&ex_uuid),
                "scenario YAML should be cached"
            );

            assert!(
                shared.scenario_docker_cache.contains_key(&ex_uuid),
                "scenario docker image and command should be cached"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn mark_execution_complete_evicts_resolved_scenario_cache() {
        let (mut eq, h, _, _) = EventQueue::new(1, 5);
        let run_uuid = Uuid::new_v4();
        let ex_id = Uuid::new_v4();

        h.with_shared(|shared| {
            shared.register_execution(ex_id, run_uuid);
            shared
                .resolved_scenario_cache
                .insert(ex_id, "yaml".to_string());
        })
        .await;
        eq.inner.lock().await.running_executions.insert(ex_id);

        eq.mark_execution_complete(ex_id).await;

        h.with_shared(|shared| {
            assert!(
                !shared.resolved_scenario_cache.contains_key(&ex_id),
                "scenario cache entry should be evicted"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn resolve_environment_for_execution_returns_error_when_cache_empty() {
        let (_, _, eqs, _) = EventQueue::new(1, 5);
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eqs.resolve_environment_for_execution(&ex).await;

        assert!(
            matches!(res, Err(ResolverError::UnknownExecution(_))),
            "expected UnknownExecution, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn resolve_scenario_for_execution_returns_error_when_cache_empty() {
        let (_, _, eqs, _) = EventQueue::new(1, 5);
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        let res = eqs.resolve_scenario_for_execution(&ex).await;

        assert!(
            matches!(res, Err(ResolverError::UnknownExecution(_))),
            "expected UnknownExecution, got: {res:?}"
        );
    }

    #[tokio::test]
    async fn send_to_resolver_forwards_resolve_env_config() {
        let (eq, _, _, mut rx) = EventQueue::new(1, 5);
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        eq.send_to_resolver(ResolverInput::ResolveConfig(ex.clone()))
            .unwrap();

        let input = rx.try_recv().expect("ResolverInput should be forwarded");
        assert!(
            matches!(input, ResolverInput::ResolveConfig(ref e) if e.uuid() == ex.uuid()),
            "wrong input forwarded: {input:?}"
        );
    }

    #[tokio::test]
    async fn evict_resolved_env_config_removes_cache_entry() {
        let (eq, h, _, _) = EventQueue::new(1, 5);
        let ex_id = Uuid::new_v4();

        h.with_shared(|shared| {
            shared.resolved_env_cache.insert(ex_id, "yaml".to_string());
        })
        .await;

        eq.evict_resolved_env_config(ex_id).await;

        h.with_shared(|shared| {
            assert!(
                !shared.resolved_env_cache.contains_key(&ex_id),
                "env cache entry should be evicted"
            );
        })
        .await;
    }
}
