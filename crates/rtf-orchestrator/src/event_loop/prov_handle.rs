use crate::{
    context::OrchestratorContext,
    db::{ClusterId, PoolId, TestExecution, TestRun, UpdateHandle},
    event_loop::{
        Event, EventData, PendingProvision, QueueEvent,
        shared::{ResolvedExecutionConfig, RunState, Shared},
    },
    resolver::{self, ResolverError},
};
use rtf_config::{
    StableSource,
    checks::Check,
    inlining::{Inline, InlineMode},
    templating::Template,
};
use rtf_orchestrator_shared::{
    OutputCollectionResponse, PrometheusQueries, test_plan::OrchestratorTestPlan,
    workload_config::WorkloadConfig,
};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use tracing::error;
use uuid::Uuid;

/// A handle for submitting provisioning requests to the `EventQueue`.
#[derive(Debug, Clone)]
pub struct ProvisioningHandle {
    /// Shared state with the parent event queue
    shared: Arc<Mutex<Shared>>,
    /// Sender for submitting provisioning events to the event queue
    tx: UnboundedSender<QueueEvent>,
}

impl ProvisioningHandle {
    pub(super) fn new(shared: Arc<Mutex<Shared>>, tx: UnboundedSender<QueueEvent>) -> Self {
        Self { shared, tx }
    }

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
        workload_config: WorkloadConfig,
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
                    workload_config,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, DEFAULT_POOL, WorkloadClusters},
        db::MockUpdateHandle,
        event_loop::{EventQueue, tests::stub_test_plan},
    };
    use rtf_orchestrator_shared::payload::SourceKeyedArrayMap;
    use std::{assert_matches, collections::HashMap};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    fn empty_source_map<T>() -> SourceKeyedArrayMap<T> {
        SourceKeyedArrayMap {
            keys: vec![],
            data: vec![],
        }
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

    fn test_prov_handle() -> (ProvisioningHandle, UnboundedReceiver<QueueEvent>) {
        let cfg = WorkloadClusters::for_test();
        let shared = Arc::new(Mutex::new(Shared {
            runs: HashMap::new(),
            executions: HashMap::new(),
            max_queued_executions: cfg.max_queued_executions,
            n_queued: 0,
        }));
        let (tx, rx) = unbounded_channel();
        let h = ProvisioningHandle::new(shared, tx);

        (h, rx)
    }

    #[tokio::test]
    async fn request_provisioning_happy_path() {
        let (h, mut rx) = test_prov_handle();
        let tr = TestRun::create_stub(1, "test");
        let mut mock = MockUpdateHandle::with_run(tr.clone());
        cache_stub_run(&h, tr.uuid()).await;
        h.shared.lock().await.n_queued = 1;

        let res = h
            .request_provisioning(&tr, "test", 0, DEFAULT_POOL, &mut mock)
            .await;

        assert_matches!(
            res,
            Ok(true),
            "should have submitted the execution: {res:?}"
        );

        h.with_shared(|shared| {
            assert_eq!(shared.n_queued, 0, "n_queued should have been decremented")
        })
        .await;

        let event = rx.try_recv().expect("event should have been sent");
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
    async fn request_provisioning_returns_false_when_channel_is_closed() {
        let (h, rx) = test_prov_handle();
        let tr = TestRun::create_stub(1, "test");
        let mut mock = MockUpdateHandle::with_run(tr.clone());
        cache_stub_run(&h, tr.uuid()).await;
        h.shared.lock().await.n_queued = 1;
        drop(rx);

        let res = h
            .request_provisioning(&tr, "test", 0, DEFAULT_POOL, &mut mock)
            .await;

        assert!(res.is_err(), "should have failed to send event");
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
}
