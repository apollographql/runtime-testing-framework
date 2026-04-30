use crate::{
    config::Config,
    conn,
    context::RepContext,
    db::{TestRun, UpdateHandle},
    event_loop::ProvisioningHandle,
    state::TestRunWithPayload,
};
use rep_orchestrator_shared::{payload::TriggerPayload, test_plan::RepTestPlan};
use rtf_config::{
    StableSource,
    checks::Check,
    context::ResolutionContext,
    templating::{Template, TemplateContext},
};
use std::{collections::HashMap, mem::take, ops::ControlFlow};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info_span, warn};
use uuid::Uuid;

const MSG_RUN_CHECKS: &str = "running test plan static checks";

/// A long lived Tokio task that is responsible for running all RTF related logic that executes
/// within the server.
///
/// See `resolve_variant` below for the specific [rtf_config] logic that is run on the server.
///
/// # Differences compared to `rtf_cli`
/// The resolution logic used here only supports processing a [RepTestPlan] that has been submitted
/// as part of a [TriggerPayload] (prepared using `rtf rep prepare` on the command line). That
/// preparation logic handles all filesystem operations on the client side and provides the
/// required local file data for us to construct a [RepContext] that can then handle resolving
/// what's left.
pub async fn resolver_task(
    mut rx: UnboundedReceiver<TestRunWithPayload>,
    prov_handle: ProvisioningHandle,
) -> crate::Result<()> {
    while let Some(TestRunWithPayload { test_run, payload }) = rx.recv().await {
        match resolve_test_plan(test_run, payload, Config::get(), conn!(), &prov_handle).await {
            ControlFlow::Break(_) => break,
            ControlFlow::Continue(_) => continue,
        }
    }

    warn!("resolver channel closed, exiting");

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ResolverError {
    #[error("event loop channel closed")]
    EventChannelClosed,

    #[error("inlining failed: {0}")]
    Inlining(#[from] rtf_config::inlining::Errors),

    #[error("unable to expand matrix variants: {0}")]
    MatrixExpansion(#[from] rtf_config::formats::Error),

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

async fn resolve_test_plan<H: UpdateHandle>(
    test_run: TestRun,
    payload: TriggerPayload,
    cfg: &Config,
    update_handle: &mut H,
    prov_handle: &ProvisioningHandle,
) -> ControlFlow<()> {
    let span = info_span!("resolve", test_run_id = %test_run.uuid(), name = %test_run.name());
    let _guard = span.enter();

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
    payload: TriggerPayload,
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
        .cache_for_test_run(test_run.uuid(), ctx, test_plan)
        .await;

    let mut n_submitted = 0;
    for (i, (name, _)) in expanded_matrix.iter().enumerate() {
        let ex = match update_handle.init_execution(test_run, name, i).await {
            Ok(ex) => ex,
            Err(e) => {
                error!(%e, %name, "unable to initialise execution in DB");
                continue;
            }
        };

        n_submitted += 1;
        prov_handle
            .request_provisioning(ex, test_run.uuid())
            .await?;
    }

    // If we failed to init any executions then there's nothing to clear the cache later, so we
    // clear it now and mark the run as unrunnable.
    if n_submitted == 0 {
        prov_handle.evict_payload_cache(test_run.uuid()).await;
        update_handle
            .clear_cached_payload_for_run(test_run.uuid())
            .await;
        update_handle
            .mark_run_as_unrunnable(test_run, "unable to initialise executions".into())
            .await;
    }

    Ok(())
}

fn prepare_resolution(cfg: &Config, payload: TriggerPayload) -> Result<(RepContext, RepTestPlan)> {
    let TriggerPayload {
        mut test_plan,
        relative_files,
        custom_providers,
    } = payload;

    let ctx = RepContext::new(cfg, relative_files, custom_providers);

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
    };
    use indoc::indoc;
    use rep_orchestrator_shared::{
        payload::{SourceKeyedArrayMap, TriggerPayload},
        test_plan::RepTestPlan,
    };
    use rtf_config::formats::{
        DockerCommand, DockerComposeEnvironment, DockerScenario, EnvironmentConfig, Matrix,
        ScenarioConfig,
    };
    use rtf_config::providers::file::compose::NamedComposeFileProvider;
    use rtf_config::templating::{Field, Scalar};
    use simple_test_case::test_case;

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
            mgmt_context: Some("dummy".to_string()),
            workload_context: "dummy".to_string(),
            orchestrator_url: "http://localhost:8035".to_string(),
            gcs_bucket: "test-bucket".to_string(),
            gcs_url_ttl_secs: 300,
            mock_internal_gcs_url: Some("http://mock-gcs-internal".to_string()),
            mock_public_gcs_url: Some("http://mock-gcs-public".to_string()),
        }
    }

    fn minimal_rep_test_plan() -> RepTestPlan {
        RepTestPlan {
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
                },
            },
            environment: EnvironmentConfig {
                name: "test environment".to_string(),
                description: "test".to_string(),
                variable_definitions: vec![],
                custom_providers: vec![],
                execution: DockerComposeEnvironment {
                    project_name: None,
                    compose_files: vec![],
                    file_providers: vec![],
                    env_vars: HashMap::new(),
                },
            },
        }
    }

    fn empty_payload() -> TriggerPayload {
        TriggerPayload {
            test_plan: minimal_rep_test_plan(),
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

    fn payload_with_conflicting_var() -> TriggerPayload {
        // Conflicting key in both variables and matrix dimensions triggers TemplatingCheck
        let mut test_plan = minimal_rep_test_plan();
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
            include: vec![],
        };

        TriggerPayload {
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

    fn payload_with_bad_variant_names() -> TriggerPayload {
        // variant_names references a variable not in dimensions → MatrixExpansion fails
        let mut test_plan = minimal_rep_test_plan();
        test_plan.matrix = Matrix {
            variant_names: Some("${nonexistent}".to_string()),
            dimensions: [("a".to_string(), vec![Scalar::String("val1".to_string())])].into(),
            include: vec![],
        };

        TriggerPayload {
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
    async fn resolve_test_plan_set_resolving_fails_exits_early() {
        let test_run = TestRun::create_stub(1, "test");
        // Empty handle — test_run is not registered, so update_test_run_status will fail
        let mut handle = MockUpdateHandle::default();
        let cfg = dummy_config();
        let (_eq, ph, _, _) =
            EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

        _ = resolve_test_plan(test_run, empty_payload(), &cfg, &mut handle, &ph).await;

        assert_eq!(handle.status_updates, vec![]);
    }

    #[test_case(payload_with_conflicting_var(), "templating pre-check failed:"; "templating_check_fails")]
    #[test_case(payload_with_bad_variant_names(), "unable to expand matrix variants:"; "matrix_expansion_fails")]
    #[tokio::test]
    async fn resolve_test_plan_prepare_failure_sets_run_unrunnable(
        payload: TriggerPayload,
        expected_msg: &str,
    ) {
        let test_run = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&test_run);
        let cfg = dummy_config();
        let (_eq, ph, _, _) =
            EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

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
        let cfg = dummy_config();
        let (_eq, ph, _, _) =
            EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

        // compose_files with a required provider — try_check always fails for RequiredFile
        let required_compose: NamedComposeFileProvider = serde_yaml::from_str(indoc!(
            r#"
                name: required-compose
                kind: required
                message: must provide a compose file
            "#
        ))
        .expect("required compose file provider must deserialize");
        let mut test_plan = minimal_rep_test_plan();
        test_plan.environment.execution.compose_files = vec![required_compose];
        let payload = TriggerPayload {
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
        let cfg = dummy_config();
        let (mut eq, ph, eqs, mut rx) =
            EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

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
        let TestRunWithPayload { test_run, payload } = rx.recv().await.unwrap();

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
        // time (when CreateEnvConfigMap is dispatched), so no execution status updates are
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

        // One CreateEnvConfigMap event sent
        let evt = eq.next_event().await.unwrap();
        assert!(
            matches!(evt.data, EventData::CreateEnvConfigMap),
            "expected CreateEnvConfigMap event"
        );
        assert!(eq.is_empty(), "only one event expected");
    }

    #[tokio::test]
    async fn resolve_test_plan_evicts_cache_when_all_init_executions_fail() {
        let tr = TestRun::create_stub(1, "test");

        // No `test_runs` registered on the handle: init_execution will fail with
        // `UnknownTestRun` for every variant.
        let mut handle = MockUpdateHandle::default();
        let cfg = dummy_config();
        let (_eq, ph, _, _) =
            EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

        _ = resolve_test_plan(tr.clone(), empty_payload(), &cfg, &mut handle, &ph).await;

        assert_eq!(handle.test_executions, vec![]);
        assert_eq!(handle.cleared_payload_caches, vec![tr.uuid()]);
    }

    #[tokio::test]
    async fn resolve_test_plan_event_loop_closed_stops_processing() {
        let tr = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&tr);
        let cfg = dummy_config();
        // dropping the receiver for the event loop so sends will fail
        let (_, ph, eqs, mut rx) =
            EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

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
        let TestRunWithPayload { test_run, payload } = rx.recv().await.unwrap();

        let res = resolve_test_plan(test_run, payload, &cfg, &mut handle, &ph).await;
        assert_eq!(
            res,
            ControlFlow::Break(()),
            "expected ControlFlow::Break to be returned"
        )
    }
}
