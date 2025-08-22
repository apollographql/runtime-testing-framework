use crate::{cli::Values, commands::get_context_and_outdir};
use anyhow::anyhow;
use rtf_config::{
    checks::{self, Check},
    context::ResolutionContext,
    formats::TestPlanConfig,
    providers::file::Source,
    templating,
};
use std::{mem::take, path::Path};
use tracing::info;

const VALUES_PATH: &str = "test-plan-values.json";
const RESOLVED_TP_PATH: &str = "resolved-test-plan.yaml";

pub async fn check_and_run_local_test_plan(
    config_file_path: &str,
    values: Values,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_outdir(out_dir)?;
    let out_dir = ctx.canonicalize_path(out_dir)?;

    info!("loading and resolving test plan");
    let test_plan = TestPlanConfig::try_load_and_resolve_from_path(config_file_path, &ctx).await?;

    check_and_run_test_plan_with_context(test_plan, values, &out_dir, ctx).await
}

pub async fn check_and_run_github_test_plan(
    org_repo_path: String,
    git_ref: Option<String>,
    values: Values,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_outdir(out_dir)?;
    let out_dir = ctx.canonicalize_path(out_dir)?;

    let (org, repo_and_path) = org_repo_path
        .split_once('/')
        .ok_or(anyhow!("invalid GitHub uri"))?;
    let (repo, path) = repo_and_path
        .split_once('/')
        .ok_or(anyhow!("invalid GitHub uri"))?;

    info!("fetching and resolving test plan from GitHub");
    let test_plan =
        TestPlanConfig::try_load_and_resolve_from_github(org, repo, path, git_ref, &ctx).await?;

    check_and_run_test_plan_with_context(test_plan, values, &out_dir, ctx).await
}

async fn check_and_run_test_plan_with_context(
    mut test_plan: TestPlanConfig,
    values: Values,
    out_dir: &Path,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    values.merge(&mut test_plan.values, &mut ctx)?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work()?;

    info!("creating output directory");
    ctx.create_dir_all(out_dir)?;

    if let Source::Local { abs_path } = test_plan.sources.test_plan() {
        let config_dir = ctx.dir_containing(abs_path);
        ctx.set_current_dir(config_dir)?;
    }

    if test_plan.matrix.is_empty() {
        info!("executing test plan");
        return run_one(test_plan, out_dir, &mut ctx).await;
    }

    let n = test_plan.n_matrix_variants();

    for (mut i, tp) in test_plan.iter_matrix_variants().enumerate() {
        i += 1;
        ctx.set_values(&tp.values);
        let sub_dir = out_dir.join(format!("matrix_variant_{i}"));
        info!("creating output directory for matrix variant {i}/{n}");
        ctx.create_dir_all(&sub_dir)?;

        info!("executing test plan {i}/{n}");
        run_one(tp, &sub_dir, &mut ctx).await?;
    }

    Ok(())
}

async fn run_one(
    mut test_plan: TestPlanConfig,
    out_dir: &Path,
    ctx: &mut impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("templating environment setup");
    let mut values = take(&mut test_plan.values);
    test_plan.try_template_environment_setup(&values)?;

    info!("checking environment setup");
    test_plan.environment.setup.command.try_check(
        &mut Vec::new(),
        test_plan.sources.environment(),
        ctx,
    )?;

    info!("executing environment setup");
    let setup_provides = test_plan.run_environment_setup(out_dir, ctx).await?;
    values.extend(setup_provides);

    info!("templating scenario and environment teardown commands");
    let mut builder = templating::ErrorBuilder::from(test_plan.try_template_scenario(&values));
    builder.append(test_plan.try_template_environment_teardown(&values));
    builder.into_result(())?;

    info!("checking scenario and environment teardown commands");
    let mut builder = checks::ErrorBuilder::from(test_plan.scenario.command.try_check(
        &mut Vec::new(),
        test_plan.sources.scenario(),
        ctx,
    ));
    builder.append(test_plan.environment.teardown.try_check(
        &mut Vec::new(),
        test_plan.sources.environment(),
        ctx,
    ));
    builder.into_result(())?;

    info!("executing scenario");
    test_plan.run_scenario(out_dir, ctx).await?;

    info!("executing environment teardown");
    test_plan.run_environment_teardown(out_dir, ctx).await?;

    info!("writing out resolved test plan and values");
    ctx.write(
        out_dir.join(VALUES_PATH),
        serde_json::to_string_pretty(&values)?,
    )?;
    ctx.write(
        out_dir.join(RESOLVED_TP_PATH),
        serde_yaml::to_string(&test_plan)?,
    )?;

    info!("done");

    Ok(())
}
