use crate::commands::{
    get_context, load_and_resolve_test_plan_from_github, load_and_resolve_test_plan_from_local,
};
use rep_orchestrator_shared::{
    payload::{SourceKeyedArrayMap, TriggerPayload},
    test_plan::RepTestPlan,
};
use rtf_config::{
    StableSource,
    context::ResolutionContext,
    formats::Sources,
    run::RunProviders,
    templating::{Template, TemplateContext},
};
use rtf_core::variables::Variables;
use std::{collections::HashMap, mem::take, sync::Arc};
use tracing::info;

pub async fn write_rep_trigger_payload_to_stdout(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<()> {
    let payload = prepare_rep_trigger_payload(test_plan_path, github, git_ref, variables).await?;
    print!("{}", serde_json::to_string_pretty(&payload)?);

    Ok(())
}

pub async fn prepare_rep_trigger_payload(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<TriggerPayload> {
    let ctx = get_context();

    info!("loading and resolving test plan");
    let (test_plan, sources) = if github {
        load_and_resolve_test_plan_from_github(test_plan_path, git_ref, &ctx).await?
    } else {
        load_and_resolve_test_plan_from_local(test_plan_path, &ctx).await?
    };

    prepare_rep_trigger_payload_with_context(test_plan, sources, variables, ctx).await
}

async fn prepare_rep_trigger_payload_with_context(
    mut test_plan: RepTestPlan,
    sources: Sources,
    variables: Variables,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<TriggerPayload> {
    let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
    ctx.set_sources(sources.with_variables_file(vars_file_src));

    let n = test_plan.matrix.n_variants();
    let mut files = HashMap::new();

    for (mut i, (_, mut variant)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;

        let variables = take(&mut variant.variables);
        let template_ctx = TemplateContext::new(
            variables,
            variable_sources.clone(),
            ctx.custom_provider_definitions(),
        );

        info!("extracting relative file providers for matrix variant {i}/{n}");
        variant.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)?;
        try_extract_relative_files(&variant, &mut files, &ctx).await?;
    }

    let custom_providers = Arc::unwrap_or_clone(ctx.custom_provider_definitions());
    let mut raw_cps = HashMap::new();
    for (k, def) in custom_providers.test_plan.into_iter() {
        raw_cps.insert((StableSource::TestPlan, k), def);
    }
    for (k, def) in custom_providers.environment.into_iter() {
        raw_cps.insert((StableSource::Environment, k), def);
    }
    for (k, def) in custom_providers.scenario.into_iter() {
        raw_cps.insert((StableSource::Scenario, k), def);
    }

    Ok(TriggerPayload {
        test_plan,
        relative_files: SourceKeyedArrayMap::from_data(files),
        custom_providers: SourceKeyedArrayMap::from_data(raw_cps),
    })
}

async fn try_extract_relative_files(
    test_plan: &RepTestPlan,
    files: &mut HashMap<(StableSource, String), String>,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<()> {
    test_plan
        .environment
        .execution
        .try_extract_relative_files(files, ctx)
        .await?;
    test_plan
        .scenario
        .execution
        .try_extract_relative_files(files, ctx)
        .await?;

    Ok(())
}
