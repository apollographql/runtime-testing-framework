use crate::{
    Error,
    config::Config,
    conn,
    context::OrchestratorContext,
    db::{ClusterId, TestExecution, TestRun, UpdateHandle},
    event_loop::{EventData, ProvisioningHandle},
    state::TestRunWithPayload,
};
use rtf_config::{
    StableSource,
    checks::Check,
    context::ResolutionContext,
    templating::{Template, TemplateContext},
};
use rtf_orchestrator_shared::{payload::PreparedPayload, test_plan::OrchestratorTestPlan};
use std::{
    collections::{HashMap, VecDeque},
    mem::take,
    ops::ControlFlow,
};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, warn};
use uuid::Uuid;

#[derive(Debug)]
pub enum ResolverInput {
    TestRun(Box<TestRunWithPayload>),
    ResolveConfig(TestExecution, ClusterId),
}

const MSG_RUN_CHECKS: &str = "running test plan static checks";

#[derive(Debug, thiserror::Error)]
pub enum ResolverError {
    #[error("event loop channel closed")]
    EventChannelClosed,

    #[error("inlining failed: {0}")]
    Inlining(#[from] rtf_config::inlining::Errors),

    #[error("unable to expand matrix variants: {0}")]
    MatrixExpansion(#[from] rtf_config::formats::Error),

    #[error("serialisation error: {0}")]
    Serialisation(String),

    #[error("templating pre-check failed: {0}")]
    TemplatingCheck(rtf_config::templating::Errors),

    #[error("{0} is not a known execution ID")]
    UnknownExecution(Uuid),

    #[error("{0} is not a known run ID")]
    UnknownRun(Uuid),

    #[error("static checks failed: {0}")]
    VariantCheck(#[from] rtf_config::checks::Errors),

    #[error("templating failed: {0}")]
    VariantTemplating(rtf_config::templating::Errors),
}

pub(crate) type Result<T> = std::result::Result<T, ResolverError>;

/// A long lived Tokio task that is responsible for running all RTF related logic that executes
/// within the server.
///
/// See `resolve_variant` below for the specific [rtf_config] logic that is run on the server.
///
/// # Differences compared to `rtf_cli`
/// The resolution logic used here only supports processing a [OrchestratorTestPlan] that has been submitted
/// as part of a [PreparedPayload] (prepared using `rtf remote prepare` on the command line). That
/// preparation logic handles all filesystem operations on the client side and provides the
/// required local file data for us to construct a [OrchestratorContext] that can then handle resolving
/// what's left.
pub async fn resolver_task(
    rx: UnboundedReceiver<ResolverInput>,
    prov_handle: ProvisioningHandle,
) -> crate::Result<()> {
    let mut queue = ResolverQueue::new(rx);

    while let Some(input) = queue.next_input().await {
        match input {
            ResolverInput::TestRun(boxed) => {
                let TestRunWithPayload { test_run, payload } = *boxed;
                match resolve_test_plan(test_run, payload, Config::get(), conn!(), &prov_handle)
                    .await
                {
                    ControlFlow::Break(_) => break,
                    ControlFlow::Continue(_) => continue,
                }
            }

            ResolverInput::ResolveConfig(test_execution, cluster) => {
                resolve_config(test_execution, cluster, &prov_handle).await;
            }
        }
    }

    warn!("resolver channel closed, exiting");

    Ok(())
}

/// Owns the resolver task's input channel and the persistent priority buckets used to order
/// resolver work over [ResolveConfig][ResolverInput::ResolveConfig] > [TestRun][ResolverInput::TestRun].
struct ResolverQueue {
    rx: UnboundedReceiver<ResolverInput>,
    test_runs: VecDeque<Box<TestRunWithPayload>>,
    resolve_configs: VecDeque<(TestExecution, ClusterId)>,
}

impl ResolverQueue {
    fn new(rx: UnboundedReceiver<ResolverInput>) -> Self {
        Self {
            rx,
            test_runs: VecDeque::new(),
            resolve_configs: VecDeque::new(),
        }
    }

    /// Returns the next [ResolverInput] to be processed, draining the channel into the priority
    /// buckets and then popping from the highest non-empty bucket in this order:
    ///
    /// 1. [ResolverInput::ResolveConfig]
    /// 2. [ResolverInput::TestRun]
    ///
    /// Buckets persist across calls. Blocks when all buckets and the channel are empty. Returns
    /// [None] only when all external senders have been dropped and all buckets are empty.
    async fn next_input(&mut self) -> Option<ResolverInput> {
        loop {
            while let Ok(input) = self.rx.try_recv() {
                self.push_input(input);
            }

            if let Some((ex, cluster)) = self.resolve_configs.pop_front() {
                return Some(ResolverInput::ResolveConfig(ex, cluster));
            }
            if let Some(boxed) = self.test_runs.pop_front() {
                return Some(ResolverInput::TestRun(boxed));
            }

            let input = self.rx.recv().await?;
            self.push_input(input);
        }
    }

    fn push_input(&mut self, input: ResolverInput) {
        match input {
            ResolverInput::TestRun(boxed) => self.test_runs.push_back(boxed),
            ResolverInput::ResolveConfig(ex, cluster) => {
                self.resolve_configs.push_back((ex, cluster))
            }
        }
    }
}

#[tracing::instrument(skip_all, fields(test_run_id = %test_run.uuid(), name = %test_run.name()))]
async fn resolve_test_plan<H: UpdateHandle>(
    test_run: TestRun,
    payload: PreparedPayload,
    cfg: &Config,
    update_handle: &mut H,
    prov_handle: &ProvisioningHandle,
) -> ControlFlow<()> {
    if let Err(e) = try_resolve(&test_run, payload, update_handle, cfg, prov_handle).await {
        match e {
            ResolverError::EventChannelClosed => {
                error!(%e);
                return ControlFlow::Break(());
            }

            _ => {
                warn!(%e, "test plan resolution failed");
                update_handle
                    .mark_run_as_unrunnable(&test_run, e.to_string())
                    .await;
            }
        }
    }

    ControlFlow::Continue(())
}

async fn try_resolve<H>(
    test_run: &TestRun,
    payload: PreparedPayload,
    update_handle: &mut H,
    cfg: &Config,
    prov_handle: &ProvisioningHandle,
) -> Result<()>
where
    H: UpdateHandle,
{
    update_handle
        .mark_run_as_resolving(test_run, MSG_RUN_CHECKS.into())
        .await;

    let (ctx, test_plan) = prepare_resolution(cfg, payload.clone())?;

    // Run full static checks up front before creating executions and pushing to the queue
    for (_, mut variant) in test_plan.try_iter_matrix_variants()? {
        let variables = take(&mut variant.variables);
        let template_ctx =
            TemplateContext::new(variables, HashMap::new(), ctx.custom_provider_definitions());

        variant
            .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
            .map_err(ResolverError::VariantTemplating)?;

        variant.try_check(&mut Vec::new(), &ctx)?;
    }

    let expanded_matrix = test_plan.matrix.try_expand(&test_plan.variables)?;

    update_handle
        .cache_payload_for_run(test_run, &payload)
        .await;
    prov_handle
        .cache_for_test_run(
            test_run.uuid(),
            test_run.initiated_by().map(|s| s.to_owned()),
            ctx,
            test_plan,
        )
        .await;

    let mut n_submitted = 0;
    let mut cancelled = false;

    for (i, (name, _)) in expanded_matrix.iter().enumerate() {
        let res = prov_handle
            .request_provisioning(
                test_run,
                name,
                i,
                test_run.workload_cluster(),
                update_handle,
            )
            .await;

        match res {
            Ok(true) => n_submitted += 1,
            Ok(false) => {
                warn!(run_uuid=%test_run.uuid(), "test run cancelled mid-submission, stopping further execution creation");
                cancelled = true;
                break;
            }
            Err(Error::Resolve(ResolverError::EventChannelClosed)) => {
                return Err(ResolverError::EventChannelClosed);
            }
            Err(e) => {
                error!(%e, %name, "unable to initialise execution in DB");
                continue;
            }
        }
    }

    // If we failed to init any executions then there's nothing to clear the cache later, so we
    // clear it now and mark as unrunnable if the run wasn't cancelled.
    if n_submitted == 0 {
        prov_handle.evict_cached_run_state(test_run.uuid()).await;
        update_handle
            .clear_cached_payload_for_run(test_run.uuid())
            .await;

        if !cancelled {
            update_handle
                .mark_run_as_unrunnable(test_run, "unable to initialise executions".into())
                .await;
        }
    }

    Ok(())
}

async fn resolve_config(
    test_execution: TestExecution,
    cluster: ClusterId,
    prov_handle: &ProvisioningHandle,
) {
    match prov_handle.resolve_and_cache_config(&test_execution).await {
        Ok(()) => {
            let _ =
                prov_handle.send_event(test_execution, cluster, EventData::CreateEnvArgoWorkflow);
        }
        // If we've errored then there is nothing in the cache so we don't need to evict anything
        // from the cache
        Err(e) => {
            warn!(%e, "ResolveEnvConfig failed");
            let _ = prov_handle.send_event(
                test_execution.clone(),
                cluster.clone(),
                EventData::MarkUnrunnable(e.to_string()),
            );
            let _ = prov_handle.send_event(test_execution, cluster, EventData::CleanupNamespace);
        }
    }
}

fn prepare_resolution(
    cfg: &Config,
    payload: PreparedPayload,
) -> Result<(OrchestratorContext, OrchestratorTestPlan)> {
    let PreparedPayload {
        mut test_plan,
        relative_files,
        custom_providers,
        ..
    } = payload;

    let ctx = OrchestratorContext::new_from_inlined_files(cfg, relative_files, custom_providers);

    test_plan
        .check_templating_will_work(&HashMap::new(), &ctx)
        .map_err(ResolverError::TemplatingCheck)?;

    Ok((ctx, test_plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        db::{MockUpdateHandle, Status, TaggedStatusUpdate, TestRun},
        event_loop::{EventData, EventQueue},
        state::TestRunWithPayload,
    };
    use indoc::indoc;
    use rtf_config::{
        formats::{
            ComposeResources, DockerCommand, DockerComposeEnvironment, DockerScenario,
            EnvironmentConfig, Matrix, OutputCollection, ScenarioConfig,
        },
        providers::file::manifest::NamedManifestFileProvider,
        templating::{Field, Scalar},
    };
    use rtf_orchestrator_shared::{
        payload::{PreparedPayload, SourceKeyedArrayMap},
        test_plan::{OrchestratorEnvironment, OrchestratorTestPlan},
    };
    use simple_test_case::test_case;

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    fn minimal_orchestrator_test_plan(
        compose_files: Vec<NamedManifestFileProvider>,
    ) -> OrchestratorTestPlan {
        OrchestratorTestPlan {
            name: "test".to_string(),
            description: "test".to_string(),
            variables: HashMap::new(),
            matrix: Matrix::default(),
            custom_providers: vec![],
            scenario: ScenarioConfig {
                name: "test scenario".to_string(),
                description: "test".to_string(),
                variable_definitions: vec![],
                custom_providers: vec![],
                execution: DockerScenario {
                    docker: DockerCommand {
                        image: Field::Resolved("test-image".to_string()),
                        tag: None,
                        command: Field::Resolved("echo test".to_string()),
                    },
                    env_vars: HashMap::new(),
                    file_providers: vec![],
                    output_collection: OutputCollection { prometheus: vec![] },
                },
            },
            environment: EnvironmentConfig {
                name: "test environment".to_string(),
                description: "test".to_string(),
                variable_definitions: vec![],
                custom_providers: vec![],
                execution: OrchestratorEnvironment::DockerCompose(DockerComposeEnvironment {
                    resources: ComposeResources {
                        project_name: None,
                        compose_files,
                    },
                    file_providers: vec![],
                    env_vars: HashMap::new(),
                    output_collection: OutputCollection { prometheus: vec![] },
                }),
            },
        }
    }

    fn empty_payload() -> PreparedPayload {
        PreparedPayload {
            variables: None,
            test_plan: minimal_orchestrator_test_plan(vec![]),
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

    fn payload_with_conflicting_var() -> PreparedPayload {
        // Conflicting key in both variables and matrix dimensions triggers TemplatingCheck
        let mut test_plan = minimal_orchestrator_test_plan(vec![]);
        test_plan
            .variables
            .insert("my_var".to_string(), Scalar::String("default".to_string()));
        test_plan.matrix = Matrix {
            variant_names: None,
            dimensions: [(
                "my_var".to_string(),
                vec![Scalar::String("val1".to_string())],
            )]
            .into(),
            compound: Default::default(),
        };

        PreparedPayload {
            variables: None,
            test_plan,
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

    fn payload_with_bad_variant_names() -> PreparedPayload {
        // variant_names references a variable not in dimensions → MatrixExpansion fails
        let mut test_plan = minimal_orchestrator_test_plan(vec![]);
        test_plan.matrix = Matrix {
            variant_names: Some("${nonexistent}".to_string()),
            dimensions: [("a".to_string(), vec![Scalar::String("val1".to_string())])].into(),
            compound: Default::default(),
        };

        PreparedPayload {
            variables: None,
            test_plan,
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

    fn mock_handle_with_run(test_run: &TestRun) -> MockUpdateHandle {
        let mut handle = MockUpdateHandle::default();
        handle.test_runs.push(test_run.clone());
        handle
    }

    #[tokio::test]
    async fn resolve_config_success_sends_create_env_argo_workflow() {
        let cfg = Config::for_test();
        let (mut eq, ph, eqs, _) = EventQueue::new(&cfg.workload_clusters);
        let tr = TestRun::create_stub(1, "test");
        let mut mock = mock_handle_with_run(&tr);

        let ctx = crate::context::OrchestratorContext::new_from_inlined_files(
            &cfg,
            rtf_orchestrator_shared::payload::SourceKeyedArrayMap {
                keys: vec![],
                data: vec![],
            },
            rtf_orchestrator_shared::payload::SourceKeyedArrayMap {
                keys: vec![],
                data: vec![],
            },
        );
        let tp = minimal_orchestrator_test_plan(vec![]);
        eqs.try_reserve_pending_executions(&tp).await.unwrap();
        ph.cache_for_test_run(tr.uuid(), None, ctx, tp).await;
        ph.request_provisioning(&tr, "test", 0, alpha_cluster(), &mut mock)
            .await
            .unwrap();
        // drain the ResolveEnvConfig sent by request_provisioning
        let _ = eq.next_event().await;

        let ex = mock.test_executions[0].clone();
        resolve_config(ex, alpha_cluster(), &ph).await;

        let evt = eq
            .next_event()
            .await
            .expect("CreateEnvArgoWorkflow should be sent");
        assert!(
            matches!(evt.data, EventData::CreateEnvArgoWorkflow),
            "expected CreateEnvArgoWorkflow, got: {:?}",
            evt.data
        );
    }

    #[tokio::test]
    async fn resolve_config_failure_sends_mark_unrunnable_and_cleanup() {
        let cfg = Config::for_test();
        let (mut eq, ph, _, _) = EventQueue::new(&cfg.workload_clusters);
        // execution NOT registered → resolve_and_cache_env_config will fail
        let ex = TestExecution::create_stub(1, 1, 0, "test");

        resolve_config(ex, alpha_cluster(), &ph).await;

        let evt1 = eq
            .next_event()
            .await
            .expect("MarkUnrunnable should be sent");
        assert!(
            matches!(evt1.data, EventData::MarkUnrunnable(_)),
            "expected MarkUnrunnable, got: {:?}",
            evt1.data
        );
        let evt2 = eq
            .next_event()
            .await
            .expect("CleanupNamespace should be sent");
        assert!(
            matches!(evt2.data, EventData::CleanupNamespace),
            "expected CleanupNamespace, got: {:?}",
            evt2.data
        );
    }

    #[tokio::test]
    async fn resolve_test_plan_set_resolving_fails_exits_early() {
        let test_run = TestRun::create_stub(1, "test");
        // Empty handle — test_run is not registered, so update_test_run_status will fail
        let mut handle = MockUpdateHandle::default();
        let cfg = Config::for_test();
        let (_eq, ph, _, _) = EventQueue::new(&cfg.workload_clusters);

        _ = resolve_test_plan(test_run, empty_payload(), &cfg, &mut handle, &ph).await;

        assert_eq!(handle.status_updates, vec![]);
    }

    #[test_case(payload_with_conflicting_var(), "templating pre-check failed:"; "templating_check_fails")]
    #[test_case(payload_with_bad_variant_names(), "unable to expand matrix variants:"; "matrix_expansion_fails")]
    #[tokio::test]
    async fn resolve_test_plan_prepare_failure_sets_run_unrunnable(
        payload: PreparedPayload,
        expected_msg: &str,
    ) {
        let test_run = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&test_run);
        let cfg = Config::for_test();
        let (_eq, ph, _, _) = EventQueue::new(&cfg.workload_clusters);

        _ = resolve_test_plan(test_run.clone(), payload, &cfg, &mut handle, &ph).await;

        let run_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Run(_, _)))
            .collect();
        assert_eq!(
            run_updates.len(),
            2,
            "expected Resolving + Unrunnable: {:?}",
            run_updates
        );
        assert!(
            matches!(run_updates[0], TaggedStatusUpdate::Run(_, s) if s.status == Status::Resolving)
        );

        let unrunnable = match run_updates[1] {
            TaggedStatusUpdate::Run(_, unrunnable) => unrunnable,
            _ => panic!("expected Run update at index 1"),
        };
        assert_eq!(unrunnable.status, Status::Unrunnable);

        let msg = unrunnable.message.as_deref().unwrap_or("");
        assert!(
            msg.contains(expected_msg),
            "expected message to contain {expected_msg:?}, got: {msg:?}"
        );
    }

    #[tokio::test]
    async fn resolve_test_plan_variant_static_check_fails_sets_run_unrunnable() {
        let test_run = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&test_run);
        let cfg = Config::for_test();
        let (_eq, ph, _, _) = EventQueue::new(&cfg.workload_clusters);

        // compose_files with a required provider — try_check always fails for RequiredFile
        let required_compose: NamedManifestFileProvider = serde_yaml::from_str(indoc!(
            r#"
                name: required-compose
                kind: required
                message: must provide a compose file
            "#
        ))
        .expect("required compose file provider must deserialize");
        let test_plan = minimal_orchestrator_test_plan(vec![required_compose]);
        let payload = PreparedPayload {
            variables: None,
            test_plan,
            relative_files: SourceKeyedArrayMap {
                keys: vec![],
                data: vec![],
            },
            custom_providers: SourceKeyedArrayMap {
                keys: vec![],
                data: vec![],
            },
        };

        _ = resolve_test_plan(test_run.clone(), payload, &cfg, &mut handle, &ph).await;

        let run_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Run(_, _)))
            .collect();
        assert_eq!(run_updates.len(), 2, "{run_updates:?}");
        assert!(
            matches!(run_updates[0], TaggedStatusUpdate::Run(_, s) if s.status == Status::Resolving)
        );
        let TaggedStatusUpdate::Run(_, unrunnable) = run_updates[1] else {
            panic!("expected Run update at index 1");
        };
        assert_eq!(unrunnable.status, Status::Unrunnable);
        let msg = unrunnable.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("static checks failed:"),
            "expected message to contain 'static checks failed:', got: {msg:?}"
        );
        assert!(
            msg.contains("must provide a compose file"),
            "expected message to contain 'must provide a compose file', got: {msg:?}"
        );

        assert_eq!(handle.test_executions, vec![]);

        let ex_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Execution(_, _)))
            .collect();

        assert!(ex_updates.is_empty(), "{ex_updates:?}");
    }

    #[tokio::test]
    async fn resolve_test_plan_success_sends_provision_event() {
        let tr = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&tr);
        let cfg = Config::for_test();
        let (mut eq, ph, eqs, mut rx) = EventQueue::new(&cfg.workload_clusters);

        // The checks we have in place around submitting test plans for resolution mean that we
        // need to ensure that we have the correct shared state before calling `resolve_test_plan`.
        // Rather than spoof it with test-only methods for manipulating that state, we call
        // `try_submit_test_plan` to drive things in the expected way.
        let payload = empty_payload();
        let claim = eqs
            .try_reserve_pending_executions(&payload.test_plan)
            .await
            .unwrap();
        eqs.try_submit_test_plan(claim, tr.clone(), payload)
            .await
            .unwrap();
        let ResolverInput::TestRun(boxed) = rx.recv().await.unwrap() else {
            panic!("expected TestRun input");
        };
        let TestRunWithPayload { test_run, payload } = *boxed;

        _ = resolve_test_plan(test_run, payload, &cfg, &mut handle, &ph).await;

        // TestRun status set to Resolving
        let run_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Run(_, _)))
            .collect();
        assert_eq!(
            run_updates.len(),
            1,
            "one run status update: {:?}",
            run_updates
        );
        assert!(
            matches!(run_updates[0], TaggedStatusUpdate::Run(_, s) if s.status == Status::Resolving)
        );

        // One execution created. Status transitions for the execution are deferred to handler
        // time (when ResolveEnvConfig is dispatched), so no execution status updates are
        // expected from the resolver itself.
        assert_eq!(handle.test_executions.len(), 1, "one execution created");
        let ex_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Execution(_, _)))
            .collect();
        assert!(
            ex_updates.is_empty(),
            "resolver should not emit execution status updates: {ex_updates:?}"
        );

        // One ResolveEnvConfig event sent
        let evt = eq.next_event().await.unwrap();
        assert!(
            matches!(evt.data, EventData::ResolveConfig),
            "expected ResolveEnvConfig event"
        );
        assert!(eq.is_empty().await, "only one event expected");
    }

    #[tokio::test]
    async fn resolve_test_plan_evicts_cache_when_all_init_executions_fail() {
        let tr = TestRun::create_stub(1, "test");

        // No `test_runs` registered on the handle: init_execution will fail with
        // `UnknownTestRun` for every variant.
        let mut handle = MockUpdateHandle::default();
        let cfg = Config::for_test();
        let (_eq, ph, _, _) = EventQueue::new(&cfg.workload_clusters);

        _ = resolve_test_plan(tr.clone(), empty_payload(), &cfg, &mut handle, &ph).await;

        assert_eq!(handle.test_executions, vec![]);
        assert_eq!(handle.cleared_payload_caches, vec![tr.uuid()]);
    }

    #[tokio::test]
    async fn resolve_test_plan_event_loop_closed_stops_processing() {
        let tr = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&tr);
        let cfg = Config::for_test();
        // dropping the receiver for the event loop so sends will fail
        let (_, ph, eqs, mut rx) = EventQueue::new(&cfg.workload_clusters);

        // The checks we have in place around submitting test plans for resolution mean that we
        // need to ensure that we have the correct shared state before calling `resolve_test_plan`.
        // Rather than spoof it with test-only methods for manipulating that state, we call
        // `try_submit_test_plan` to drive things in the expected way.
        let payload = empty_payload();
        let claim = eqs
            .try_reserve_pending_executions(&payload.test_plan)
            .await
            .unwrap();
        eqs.try_submit_test_plan(claim, tr.clone(), payload)
            .await
            .unwrap();
        let ResolverInput::TestRun(boxed) = rx.recv().await.unwrap() else {
            panic!("expected TestRun input");
        };
        let TestRunWithPayload { test_run, payload } = *boxed;

        let res = resolve_test_plan(test_run, payload, &cfg, &mut handle, &ph).await;
        assert_eq!(
            res,
            ControlFlow::Break(()),
            "expected ControlFlow::Break to be returned"
        )
    }

    fn boxed_test_run(name: &str) -> Box<TestRunWithPayload> {
        Box::new(TestRunWithPayload {
            test_run: TestRun::create_stub(1, name),
            payload: empty_payload(),
        })
    }

    #[tokio::test]
    async fn next_input_prioritises_by_kind_regardless_of_arrival_order() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut queue = ResolverQueue::new(rx);

        let env_ex = TestExecution::create_stub(1, 1, 0, "env");

        // Submit in reverse priority
        tx.send(ResolverInput::TestRun(boxed_test_run("tr")))
            .unwrap();
        tx.send(ResolverInput::ResolveConfig(
            env_ex.clone(),
            alpha_cluster(),
        ))
        .unwrap();

        let first = queue.next_input().await.unwrap();
        assert!(
            matches!(first, ResolverInput::ResolveConfig(ref ex, _) if ex.uuid() == env_ex.uuid()),
            "expected ResolveEnvConfig first, got {first:?}"
        );

        let second = queue.next_input().await.unwrap();
        assert!(
            matches!(second, ResolverInput::TestRun(_)),
            "expected TestRun second, got {second:?}"
        );
    }

    #[tokio::test]
    async fn next_input_preserves_fifo_within_a_bucket() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut queue = ResolverQueue::new(rx);

        let ex_a = TestExecution::create_stub(1, 1, 0, "a");
        let ex_b = TestExecution::create_stub(2, 1, 1, "b");
        let ex_c = TestExecution::create_stub(3, 1, 2, "c");

        tx.send(ResolverInput::ResolveConfig(ex_a.clone(), alpha_cluster()))
            .unwrap();
        tx.send(ResolverInput::ResolveConfig(ex_b.clone(), alpha_cluster()))
            .unwrap();
        tx.send(ResolverInput::ResolveConfig(ex_c.clone(), alpha_cluster()))
            .unwrap();

        for expected in [&ex_a, &ex_b, &ex_c] {
            let input = queue.next_input().await.unwrap();
            assert!(
                matches!(input, ResolverInput::ResolveConfig(ref ex, _) if ex.uuid() == expected.uuid()),
                "FIFO order broken; expected {} got {input:?}",
                expected.uuid()
            );
        }
    }

    #[tokio::test]
    async fn next_input_returns_none_when_all_senders_dropped_and_buckets_empty() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ResolverInput>();
        let mut queue = ResolverQueue::new(rx);

        drop(tx);

        let result = queue.next_input().await;
        assert!(result.is_none(), "expected None on closed channel");
    }

    #[tokio::test]
    async fn next_input_drains_buffered_events_after_senders_dropped() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut queue = ResolverQueue::new(rx);

        let env_ex = TestExecution::create_stub(1, 1, 0, "env");
        tx.send(ResolverInput::ResolveConfig(
            env_ex.clone(),
            alpha_cluster(),
        ))
        .unwrap();
        drop(tx);

        let first = queue.next_input().await.unwrap();
        assert!(
            matches!(first, ResolverInput::ResolveConfig(ref ex, _) if ex.uuid() == env_ex.uuid()),
            "expected buffered event to drain before close",
        );

        let second = queue.next_input().await;
        assert!(
            second.is_none(),
            "expected None after draining buffered event"
        );
    }
}
