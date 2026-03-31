use crate::{
    config::Config,
    conn,
    context::RepContext,
    db::{Status, TestRun, UpdateHandle},
    event_loop::{Event, EventData},
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
use std::{collections::HashMap, mem::take, ops::ControlFlow};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{error, info_span, warn};

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
    etx: UnboundedSender<Event>,
) -> crate::Result<()> {
    while let Some(TestRunWithPayload { test_run, payload }) = rx.recv().await {
        match resolve_test_plan(test_run, payload, conn!(), Config::get(), &etx).await {
            ControlFlow::Break(_) => break,
            ControlFlow::Continue(_) => continue,
        }
    }

    warn!("resolver channel closed, exiting");

    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum ResolverError {
    #[error("event loop channel closed")]
    EventChannelClosed,

    #[error("inlining failed: {0}")]
    Inlining(#[from] rtf_config::inlining::Errors),

    #[error("unable to expand matrix variants: {0}")]
    MatrixExpansion(#[from] rtf_config::formats::Error),

    #[error("unable to set test run status to Resolving: {0}")]
    SetResolvingStatus(#[from] crate::Error),

    #[error("templating pre-check failed: {0}")]
    TemplatingCheck(rtf_config::templating::Errors),

    #[error("static checks failed: {0}")]
    VariantCheck(#[from] rtf_config::checks::Errors),

    #[error("templating failed: {0}")]
    VariantTemplating(rtf_config::templating::Errors),
}

type Result<T> = std::result::Result<T, ResolverError>;

async fn resolve_test_plan<H: UpdateHandle>(
    test_run: TestRun,
    payload: TriggerPayload,
    handle: &mut H,
    cfg: &Config,
    etx: &UnboundedSender<Event>,
) -> ControlFlow<()> {
    let span = info_span!("resolve", test_run_id = %test_run.uuid(), name = %test_run.name());
    let _guard = span.enter();

    if let Err(e) = try_resolve(&test_run, payload, handle, cfg, etx).await {
        match e {
            ResolverError::EventChannelClosed => {
                error!(%e);
                return ControlFlow::Break(());
            }
            ResolverError::SetResolvingStatus(_) => {
                error!(%e);
            }
            _ => {
                warn!(%e, "test plan preparation failed");
                let _ = handle
                    .update_test_run_status(&test_run, Status::Unrunnable, Some(e.to_string()))
                    .await;
            }
        }
    }

    ControlFlow::Continue(())
}

async fn try_resolve<H: UpdateHandle>(
    test_run: &TestRun,
    payload: TriggerPayload,
    handle: &mut H,
    cfg: &Config,
    etx: &UnboundedSender<Event>,
) -> Result<()> {
    handle
        .update_test_run_status(test_run, Status::Resolving, None)
        .await?;

    let (ctx, test_plan) = prepare_resolution(cfg, payload)?;
    let variants = test_plan.try_iter_matrix_variants()?;

    for (name, mut variant) in variants {
        let ex = match handle.init_execution(test_run, &name).await {
            Ok(ex) => ex,
            Err(e) => {
                error!(%e, %name, "unable to initialise execution record");
                continue;
            }
        };

        if let Err(e) = handle
            .update_test_execution_status(&ex, Status::Resolving, None)
            .await
        {
            error!(%e, %name, "unable to set execution status to Resolving");
            continue;
        }

        if let Err(e) = resolve_variant(&mut variant, &ctx).await {
            warn!(%e, %name, "variant resolution failed");
            let _ = handle
                .update_test_execution_status(&ex, Status::Unrunnable, Some(e.to_string()))
                .await;
            continue;
        }

        if let Err(e) = etx.send(Event {
            test_execution: ex,
            data: EventData::ProvisionEnvironment(
                variant.environment.execution,
                variant.scenario.execution,
            ),
        }) {
            error!(%e, "event loop channel closed, exiting resolver");
            return Err(ResolverError::EventChannelClosed);
        }
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

async fn resolve_variant(test_plan: &mut RepTestPlan, ctx: &RepContext) -> Result<()> {
    let variables = take(&mut test_plan.variables);
    let template_ctx =
        TemplateContext::new(variables, HashMap::new(), ctx.custom_provider_definitions());

    test_plan
        .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
        .map_err(ResolverError::VariantTemplating)?;

    test_plan.try_check(&mut Vec::new(), ctx)?;

    test_plan.environment.inline(&InlineMode::All, ctx).await?;

    test_plan
        .scenario
        .execution
        .inline(&InlineMode::All, ctx)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        db::{MockUpdateHandle, Status, TaggedStatusUpdate, TestRun},
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
    use tokio::sync::mpsc::unbounded_channel;

    fn dummy_config() -> Config {
        Config {
            apollo_key: "dummy".to_string(),
            db_host: "localhost".to_string(),
            db_port: 5432,
            db_name: "test".to_string(),
            db_user: "test".to_string(),
            db_pass: "test".to_string(),
            github_token: "dummy".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8035,
            kubeconfig_path: "dummy".to_string(),
            mgmt_context: "dummy".to_string(),
            workload_context: "dummy".to_string(),
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
        let (etx, _erx) = unbounded_channel::<Event>();
        let cfg = dummy_config();

        _ = resolve_test_plan(test_run, empty_payload(), &mut handle, &cfg, &etx).await;

        assert!(
            handle.status_updates.is_empty(),
            "no status updates expected when setting Resolving fails: {:?}",
            handle.status_updates
        );
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
        let (etx, _erx) = unbounded_channel::<Event>();
        let cfg = dummy_config();

        _ = resolve_test_plan(test_run.clone(), payload, &mut handle, &cfg, &etx).await;

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
    async fn resolve_test_plan_variant_static_check_fails_sets_execution_unrunnable() {
        let test_run = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&test_run);
        let (etx, _erx) = unbounded_channel::<Event>();
        let cfg = dummy_config();

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

        _ = resolve_test_plan(test_run.clone(), payload, &mut handle, &cfg, &etx).await;

        let run_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Run(_, _)))
            .collect();
        assert_eq!(
            run_updates.len(),
            1,
            "only Resolving for the run: {:?}",
            run_updates
        );
        assert!(
            matches!(run_updates[0], TaggedStatusUpdate::Run(_, s) if s.status == Status::Resolving)
        );

        let ex_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Execution(_, _)))
            .collect();
        assert_eq!(
            ex_updates.len(),
            2,
            "expected Resolving + Unrunnable: {:?}",
            ex_updates
        );
        assert!(
            matches!(ex_updates[0], TaggedStatusUpdate::Execution(_, s) if s.status == Status::Resolving),
            "first execution update must be Resolving: {:?}",
            ex_updates[0]
        );
        let TaggedStatusUpdate::Execution(_, unrunnable) = ex_updates[1] else {
            panic!("expected Execution update at index 1");
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
    }

    #[tokio::test]
    async fn resolve_test_plan_success_sends_provision_event() {
        let test_run = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&test_run);
        let (etx, mut erx) = unbounded_channel::<Event>();
        let cfg = dummy_config();

        _ = resolve_test_plan(test_run.clone(), empty_payload(), &mut handle, &cfg, &etx).await;

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

        // One execution created and set to Resolving
        assert_eq!(handle.test_executions.len(), 1, "one execution created");
        let ex_updates: Vec<_> = handle
            .status_updates
            .iter()
            .filter(|u| matches!(u, TaggedStatusUpdate::Execution(_, _)))
            .collect();
        assert_eq!(
            ex_updates.len(),
            1,
            "one execution status update: {ex_updates:?}"
        );
        assert!(
            matches!(ex_updates[0], TaggedStatusUpdate::Execution(_, s) if s.status == Status::Resolving),
            "execution status must be Resolving: {:?}",
            ex_updates[0]
        );

        // One ProvisionEnvironment event sent
        let event = erx.try_recv().expect("one event must be sent");
        assert!(
            matches!(event.data, EventData::ProvisionEnvironment(_, _)),
            "expected ProvisionEnvironment event"
        );
        assert!(erx.try_recv().is_err(), "only one event expected");
    }

    #[tokio::test]
    async fn resolve_test_plan_event_loop_closed_stops_processing() {
        let test_run = TestRun::create_stub(1, "test");
        let mut handle = mock_handle_with_run(&test_run);
        let (etx, erx) = unbounded_channel::<Event>();
        drop(erx); // close the receiver — etx.send() will fail
        let cfg = dummy_config();

        let res = resolve_test_plan(test_run, empty_payload(), &mut handle, &cfg, &etx).await;
        assert_eq!(
            res,
            ControlFlow::Break(()),
            "expected ControlFlow::Break to be returned"
        )
    }
}
