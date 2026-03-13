//! Long lived task for resolving test plans
use crate::{
    ENV_VARS,
    context::RepContext,
    event_loop::{Event, EventType},
    rep_test_plan::RepTestPlan,
};
use rtf_config::{
    StableSource,
    checks::Check,
    context::{Context, ResolutionContext},
    formats::{EnvironmentExecution, ScenarioExecution, TestPlanConfig},
    inlining::InlineMode,
    templating::{Template, TemplateContext},
};
use std::{collections::HashMap, mem::take};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{debug, error, info, info_span, warn};
use uuid::Uuid;

#[derive(Debug)]
pub struct TestPlanWithId {
    pub id: Uuid,
    pub rtp: RepTestPlan,
}

pub async fn test_plan_resolver_task(
    mut rx: UnboundedReceiver<TestPlanWithId>,
    etx: UnboundedSender<Event>,
) {
    // TODO: work out what we want / need to do about clearing the cache of supergraph details
    let base_ctx = Context::new_from_env_vars(&ENV_VARS);

    loop {
        let TestPlanWithId {
            id,
            rtp:
                RepTestPlan {
                    mut test_plan,
                    relative_files,
                    custom_providers,
                },
        } = match rx.recv().await {
            Some(tp) => tp,
            None => {
                info!("Test plan resolver channel closed. Exiting resolver task");
                return;
            }
        };

        let span = info_span!("resolve_test_plan", %id, test_plan_name=%test_plan.name);
        let _guard = span.enter();

        let ctx = match RepContext::try_new(base_ctx.clone(), relative_files, custom_providers) {
            Ok(ctx) => ctx,
            Err(error) => {
                warn!(%error, "unable to create context");
                // TODO: mark TestRun as unrunnable
                continue;
            }
        };

        debug!("checking if templating will work");
        if let Err(error) = test_plan.check_templating_will_work(&HashMap::new(), &ctx) {
            warn!(%error, "templating will not work");
            // TODO: mark TestRun as unrunnable
            continue;
        };

        info!("expanding test plan variants");
        let it = match test_plan.try_iter_matrix_variants() {
            Ok(it) => it,
            Err(error) => {
                warn!(%error, "unable to expand matrix variants");
                // TODO: mark TestRun as unrunnable
                continue;
            }
        };

        for (name, mut variant) in it {
            debug!(%name, "resolving variant");
            if let Err(error) = resolve_variant(&mut variant, &ctx).await {
                warn!(%error, %name, "unable to resolve variant");
                // TODO: mark TestRun as unrunnable
                continue;
            }

            let environment = match variant.environment.execution {
                EnvironmentExecution::DockerCompose(inner) => inner,
                // FIXME: Need to enforce this invariant in the axum handler
                EnvironmentExecution::Script(_) => {
                    panic!("got a test plan with a script environment")
                }
            };

            let scenario = match variant.scenario.execution {
                ScenarioExecution::Docker(inner) => inner,
                // FIXME: Need to enforce this invariant in the axum handler
                ScenarioExecution::Script(_) => panic!("got a test plan with a script scenario"),
            };

            if let Err(error) = etx.send(Event {
                execution_id: Uuid::new_v4(),
                ty: EventType::ProvisionEnvironment(environment, scenario),
            }) {
                error!(%error, "unable to send test plan to event loop. exiting.");
                return;
            };
        }
    }
}

async fn resolve_variant(test_plan: &mut TestPlanConfig, ctx: &RepContext) -> anyhow::Result<()> {
    debug!("creating templating context");
    let variables = take(&mut test_plan.variables);
    let template_ctx =
        TemplateContext::new(variables, HashMap::new(), ctx.custom_provider_definitions());

    info!("templating variant");
    test_plan.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)?;

    info!("checking variant");
    test_plan.try_check(&mut Vec::new(), ctx)?;

    info!("inlining environment file providers");
    test_plan.environment.inline(&InlineMode::All, ctx).await?;

    info!("inlining scenario file providers");
    test_plan.scenario.inline(&InlineMode::All, ctx).await?;

    Ok(())
}
