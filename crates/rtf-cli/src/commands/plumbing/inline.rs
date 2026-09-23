use crate::commands::{
    get_context_and_check_outdir, load_and_resolve_test_plan_from_github,
    load_and_resolve_test_plan_from_local,
};
use rtf_config::{
    StableSource,
    context::ResolutionContext,
    formats::{PrepareOnlyTestPlanConfig, Sources},
    inlining::{self, Inline, InlineMode, InlinedProvider},
    templating::{Template, TemplateContext},
};
use rtf_core::variables::Variables;
use std::{collections::HashMap, path::Path};
use tracing::info;

const INLINED_TEST_PLAN_PATH: &str = "inlined-test-plan.yaml";

pub async fn inline_test_plan(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
    mode: InlineMode,
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
    inline_file_providers_with_context(test_plan, sources, mode, variables, ctx, outdir).await
}

async fn inline_file_providers_with_context(
    mut test_plan: PrepareOnlyTestPlanConfig,
    sources: Sources,
    mode: InlineMode,
    variables: Variables,
    mut ctx: impl ResolutionContext,
    outdir: &str,
) -> anyhow::Result<()> {
    let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
    ctx.set_sources(sources.with_variables_file(vars_file_src));

    info!("creating output directory for inlined test plan templates");
    ctx.create_dir_all(outdir)?;
    let outdir = ctx.canonicalize_path(outdir)?;
    ctx.set_output_path(&outdir);

    let mut inline_cache = HashMap::new();

    if test_plan.matrix.is_empty() {
        info!("inlining relative file providers for test plan");

        return inline_one(
            &mut test_plan,
            mode,
            &variable_sources,
            &outdir,
            None,
            &mut ctx,
            &mut inline_cache,
        )
        .await;
    }

    let n = test_plan.matrix.n_variants();

    for (mut i, (variant_name, mut variant)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;
        info!("inlining relative file providers for matrix variant {i}/{n}");
        inline_one(
            &mut variant,
            mode,
            &variable_sources,
            &outdir,
            Some(variant_name),
            &mut ctx,
            &mut inline_cache,
        )
        .await?;
    }

    Ok(())
}

async fn inline_file_providers(
    test_plan: &mut PrepareOnlyTestPlanConfig,
    mode: InlineMode,
    ctx: &mut impl ResolutionContext,
    template_variables: &HashMap<String, StableSource>,
    cache: &mut HashMap<u64, InlinedProvider>,
) -> inlining::Result<()> {
    let mut errs = inlining::ErrorBuilder::new();

    let template_ctx = TemplateContext::new(
        test_plan.variables.clone(),
        template_variables.clone(),
        ctx.custom_provider_definitions(),
    );

    info!("templating test plan");
    errs.append(
        test_plan
            .try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)
            .map_err(Into::into),
    );

    // Inline all file providers after templating
    info!("inlining file providers for test plan");
    errs.append(test_plan.scenario.try_inline(mode, ctx, cache).await);
    errs.append(test_plan.environment.try_inline(mode, ctx, cache).await);

    errs.into_result(())
}

async fn inline_one(
    test_plan: &mut PrepareOnlyTestPlanConfig,
    mode: InlineMode,
    template_variables: &HashMap<String, StableSource>,
    outdir: &Path,
    test_plan_name: Option<String>,
    ctx: &mut impl ResolutionContext,
    inline_cache: &mut HashMap<u64, InlinedProvider>,
) -> anyhow::Result<()> {
    inline_file_providers(test_plan, mode, ctx, template_variables, inline_cache).await?;

    info!("writing out inlined test plan");
    let output_path = match test_plan_name {
        Some(name) => outdir.join(format!("{name}-{INLINED_TEST_PLAN_PATH}")),
        None => outdir.join(INLINED_TEST_PLAN_PATH),
    };
    ctx.write(output_path, serde_yaml::to_string(test_plan)?)?;

    info!("done");

    Ok(())
}
