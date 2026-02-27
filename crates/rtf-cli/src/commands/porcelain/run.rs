use crate::{
    cli::{RunTarget, Variables},
    commands::{
        get_context_and_check_outdir, load_and_resolve_test_plan_from_github,
        load_and_resolve_test_plan_from_local,
    },
};
use rtf_config::{
    checks::Check,
    context::ResolutionContext,
    formats::TestPlanConfig,
    providers::file::SourceDir,
    templating::{Template, TemplateContext},
};
use std::{
    collections::HashMap,
    env::current_dir,
    mem::take,
    path::{Path, PathBuf},
};
use tracing::info;

const VARIABLES_PATH: &str = "test-plan-variables.json";
const RESOLVED_TP_PATH: &str = "resolved-test-plan.yaml";

pub async fn check_and_run_test_plan(
    test_plan_path: &str,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
    run_target: RunTarget,
    out_dir: &str,
    force: bool,
) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_check_outdir(out_dir, force)?;
    let cwd = current_dir()?;

    info!("loading and resolving test plan");
    let test_plan = if github {
        load_and_resolve_test_plan_from_github(test_plan_path, git_ref, &ctx).await?
    } else {
        load_and_resolve_test_plan_from_local(test_plan_path, &ctx).await?
    };

    check_and_run_test_plan_with_context(test_plan, variables, &out_dir, cwd, run_target, ctx).await
}

async fn check_and_run_test_plan_with_context(
    mut test_plan: TestPlanConfig,
    variables: Variables,
    out_dir: &Path,
    cwd: PathBuf,
    run_target: RunTarget,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    let variable_sources = variables.merge(&mut test_plan, &SourceDir::local(cwd), &mut ctx)?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work(&variable_sources)?;

    info!("creating output directory");
    ctx.create_dir_all(out_dir)?;
    let out_dir = ctx.canonicalize_path(out_dir)?;
    ctx.set_output_path(&out_dir);

    if test_plan.matrix.is_empty() {
        info!("executing test plan");
        return run_one(
            test_plan,
            &out_dir,
            &variable_sources,
            &run_target,
            &mut ctx,
        )
        .await;
    }

    let n = test_plan.matrix.n_variants();

    for (mut i, (name, tp)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;
        let sub_dir = out_dir.join(name);
        info!("creating output directory for matrix variant {i}/{n}");
        ctx.create_dir_all(&sub_dir)?;

        info!("executing test plan {i}/{n}");
        run_one(tp, &sub_dir, &variable_sources, &run_target, &mut ctx).await?;
    }

    Ok(())
}

async fn run_one(
    mut test_plan: TestPlanConfig,
    out_dir: &Path,
    variable_sources: &HashMap<String, SourceDir>,
    run_target: &RunTarget,
    ctx: &mut impl ResolutionContext,
) -> anyhow::Result<()> {
    let variables = take(&mut test_plan.variables);
    let source = test_plan.sources.test_plan().clone();
    let template_ctx = TemplateContext::new(
        variables,
        test_plan.sources.test_plan().clone(),
        variable_sources.clone(),
        test_plan.sources.custom_providers(),
    );

    info!("templating test plan");
    test_plan.try_template(&mut Vec::new(), &source, &template_ctx)?;

    info!("checking test plan");
    test_plan.try_check(&mut Vec::new(), ctx)?;

    let (run_setup, run_scenario, run_teardown) = run_target.as_flags();

    if run_setup {
        info!("executing environment setup");
        test_plan.run_environment_setup(out_dir, ctx).await?;
    }

    if run_scenario {
        info!("executing scenario");
        test_plan.run_scenario(out_dir, ctx).await?;
    }

    if run_teardown {
        info!("executing environment teardown");
        test_plan.run_environment_teardown(out_dir, ctx).await?;
    }

    info!("writing out resolved test plan and variables");
    ctx.write(
        out_dir.join(VARIABLES_PATH),
        serde_json::to_string_pretty(template_ctx.variables())?,
    )?;
    ctx.write(
        out_dir.join(RESOLVED_TP_PATH),
        test_plan.as_yaml_string_without_sources()?,
    )?;

    info!("done");

    Ok(())
}
