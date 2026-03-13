use crate::{
    cli::Variables,
    commands::{
        get_context_and_check_outdir, load_and_resolve_test_plan_from_github,
        load_and_resolve_test_plan_from_local,
    },
};
use anyhow::bail;
use rtf_config::{
    StableSource,
    context::ResolutionContext,
    formats::{
        EnvironmentExecution, RepTestPlan, ScenarioExecution, SourceKeyedArrayMap, Sources,
        TestPlanConfig,
    },
    templating::{Template, TemplateContext},
};
use std::{collections::HashMap, mem::take, sync::Arc};
use tracing::info;

const REP_TEST_PLAN_PATH: &str = "rep-test-plan.json";

pub async fn prepare_rep_test_plan(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
    outdir: &str,
    force: bool,
) -> anyhow::Result<()> {
    let (ctx, _outdir) = get_context_and_check_outdir(outdir, force)?;

    info!("loading and resolving test plan");
    let (test_plan, sources) = if github {
        load_and_resolve_test_plan_from_github(test_plan_path, git_ref, &ctx).await?
    } else {
        load_and_resolve_test_plan_from_local(test_plan_path, &ctx).await?
    };

    prepare_rep_test_plan_with_context(test_plan, sources, variables, ctx, outdir).await
}

async fn prepare_rep_test_plan_with_context(
    mut test_plan: TestPlanConfig,
    sources: Sources,
    variables: Variables,
    mut ctx: impl ResolutionContext,
    outdir: &str,
) -> anyhow::Result<()> {
    let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
    ctx.set_sources(sources.with_variables_file(vars_file_src));

    validate_test_plan_types(&test_plan)?;

    info!("creating output directory");
    ctx.create_dir_all(outdir)?;
    let outdir = ctx.canonicalize_path(outdir)?;
    ctx.set_output_path(&outdir);

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
        variant.try_extract_relative_files(&mut files, &ctx).await?;
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

    ctx.write(
        outdir.join(REP_TEST_PLAN_PATH),
        serde_json::to_string_pretty(&RepTestPlan {
            test_plan,
            relative_files: SourceKeyedArrayMap::from_data(files),
            custom_providers: SourceKeyedArrayMap::from_data(raw_cps),
        })?,
    )?;

    Ok(())
}

fn validate_test_plan_types(test_plan: &TestPlanConfig) -> anyhow::Result<()> {
    let mut errors: Vec<String> = Vec::new();

    if !matches!(
        test_plan.environment.execution,
        EnvironmentExecution::DockerCompose(_)
    ) {
        errors.push(
            "environment must be DockerComposeEnvironment, not a script environment".to_string(),
        );
    }

    if !matches!(test_plan.scenario.execution, ScenarioExecution::Docker(_)) {
        errors.push("scenario must be DockerScenario, not a script scenario".to_string());
    }

    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }

    Ok(())
}
