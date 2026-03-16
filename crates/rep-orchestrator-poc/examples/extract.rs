use rtf_cli::{
    cli::Variables,
    commands::{get_context_and_check_outdir, load_and_resolve_test_plan_from_local},
};
use rtf_config::{
    Rep, StableSource,
    context::ResolutionContext,
    formats::{RepPayload, RepTestPlan, SourceKeyedArrayMap, Sources},
    templating::{Template, TemplateContext},
};
use std::{
    collections::HashMap,
    env::{self},
    mem::take,
    sync::Arc,
};
use tracing::info;

const REP_TEST_PLAN_PATH: &str = "rep-test-plan.json";
const OUTDIR: &str = "output";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let test_plan_path = env::args().nth(1).expect("need a test plan path");
    let (ctx, _outdir) = get_context_and_check_outdir(OUTDIR, false)?;
    let variables = Variables {
        var: Vec::new(),
        vars: None,
    };

    info!("loading and resolving test plan");
    let (test_plan, sources) =
        load_and_resolve_test_plan_from_local::<Rep>(&test_plan_path, &ctx).await?;
    extract_relative_files_with_context(test_plan, sources, variables, ctx, OUTDIR).await
}

async fn extract_relative_files_with_context(
    mut test_plan: RepTestPlan,
    sources: Sources,
    variables: Variables,
    mut ctx: impl ResolutionContext,
    outdir: &str,
) -> anyhow::Result<()> {
    let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
    ctx.set_sources(sources.with_variables_file(vars_file_src));

    info!("creating output directory");
    ctx.create_dir_all(outdir)?;
    let outdir = ctx.canonicalize_path(outdir)?;
    ctx.set_output_path(&outdir);

    let n = test_plan.matrix.n_variants();
    let mut files = HashMap::new();

    // Extract file providers
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

    // Extract custom provider definitions (so we dedupe providers used in different files)
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
        serde_json::to_string_pretty(&RepPayload {
            test_plan,
            relative_files: SourceKeyedArrayMap::from_data(files),
            custom_providers: SourceKeyedArrayMap::from_data(raw_cps),
        })?,
    )?;

    Ok(())
}
