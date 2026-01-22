use crate::{
    cli::Variables,
    commands::{
        get_context_and_check_outdir, load_and_resolve_test_plan_from_github,
        load_and_resolve_test_plan_from_local,
    },
};
use rtf_config::{
    SourceDir,
    context::ResolutionContext,
    formats::TestPlanConfig,
    inlining,
    templating::{Template, TemplateContext},
};
use std::{
    collections::HashMap,
    env::current_dir,
    path::{Path, PathBuf},
};
use tracing::info;

const INLINED_TEST_PLAN_PATH: &str = "inlined-test-plan.yaml";

/// The subcommand that the inline command runs
/// This enum exists so the command args don't need to be propagated down through
/// all of the inline methods
pub enum InlineMode {
    All,
    RelativeFiles,
}

pub async fn inline_test_plan(
    test_plan_path: &str,
    mode: InlineMode,
    outdir: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<()> {
    let cwd = current_dir()?;
    let (ctx, _outdir) = get_context_and_check_outdir(outdir)?;

    info!("loading and resolving test plan");
    let test_plan = if github {
        load_and_resolve_test_plan_from_github(test_plan_path, git_ref, &ctx).await?
    } else {
        load_and_resolve_test_plan_from_local(test_plan_path, &ctx).await?
    };

    inline_file_providers_with_context(test_plan, mode, variables, ctx, cwd, outdir).await
}

async fn inline_file_providers_with_context(
    mut test_plan: TestPlanConfig,
    mode: InlineMode,
    variables: Variables,
    mut ctx: impl ResolutionContext,
    cwd: PathBuf,
    outdir: &str,
) -> anyhow::Result<()> {
    let variable_sources = variables.merge(&mut test_plan, &SourceDir::local(cwd), &mut ctx)?;

    info!("creating output directory for inlined test plan templates");
    ctx.create_dir_all(outdir)?;
    let outdir = ctx.canonicalize_path(outdir)?;
    ctx.set_output_path(&outdir);

    if test_plan.matrix.is_empty() {
        info!("inlining relative file providers for test plan");

        return inline_one(
            &mut test_plan,
            &mode,
            &variable_sources,
            &outdir,
            None,
            &mut ctx,
        )
        .await;
    }

    let n = test_plan.matrix.n_variants();

    for (mut i, (variant_name, mut variant)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;
        info!("inlining relative file providers for matrix variant {i}/{n}");
        inline_one(
            &mut variant,
            &mode,
            &variable_sources,
            &outdir,
            Some(variant_name),
            &mut ctx,
        )
        .await?;
    }

    Ok(())
}

async fn inline_file_providers(
    test_plan: &mut TestPlanConfig,
    mode: &InlineMode,
    ctx: &mut impl ResolutionContext,
    template_variables: &HashMap<String, SourceDir>,
) -> inlining::Result<()> {
    let mut errs = inlining::ErrorBuilder::new();

    let source = test_plan.sources.test_plan().clone();
    let template_ctx = TemplateContext::new(
        test_plan.variables.clone(),
        source.clone(),
        template_variables.clone(),
        test_plan.sources.custom_providers(),
    );

    info!("templating test plan");
    errs.append(
        test_plan
            .try_template(&mut Vec::new(), &source, &template_ctx)
            .map_err(Into::into),
    );

    match mode {
        InlineMode::All => {
            // Inline all file providers after templating
            info!("inlining file providers for test plan");
            errs.append(test_plan.scenario.inline(ctx).await);
            errs.append(test_plan.environment.inline(ctx).await);
        }
        InlineMode::RelativeFiles => {
            // Inline relative file providers after templating to ensure relative file paths that might be used in the template are resolved
            info!("inlining relative file providers for test plan");
            errs.append(test_plan.scenario.inline_all_relative_paths(ctx).await);
            errs.append(test_plan.environment.inline_all_relative_paths(ctx).await);
        }
    };

    errs.into_result(())
}

async fn inline_one(
    test_plan: &mut TestPlanConfig,
    mode: &InlineMode,
    template_variables: &HashMap<String, SourceDir>,
    outdir: &Path,
    test_plan_name: Option<String>,
    ctx: &mut impl ResolutionContext,
) -> anyhow::Result<()> {
    inline_file_providers(test_plan, mode, ctx, template_variables).await?;

    info!("writing out inlined test plan");
    let output_path = match test_plan_name {
        Some(name) => outdir.join(format!("{name}-{INLINED_TEST_PLAN_PATH}")),
        None => outdir.join(INLINED_TEST_PLAN_PATH),
    };
    ctx.write(output_path, test_plan.as_yaml_string_without_sources()?)?;

    info!("done");

    Ok(())
}
