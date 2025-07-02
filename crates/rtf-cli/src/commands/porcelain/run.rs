use crate::{cli::Values, commands::get_context_and_outdir};
use rtf_config::{
    checks::{self, Check},
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating,
};
use std::{mem::take, path::Path};
use tracing::info;

pub async fn check_and_run_test_plan(
    config_file_path: &str,
    values: Values,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_outdir(out_dir)?;
    let out_dir = ctx.canonicalize_path(out_dir)?;

    check_and_run_test_plan_with_context(config_file_path, values, &out_dir, ctx).await
}

async fn check_and_run_test_plan_with_context(
    path: &str,
    values: Values,
    out_dir: &Path,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let mut test_plan = TestPlanConfig::try_load_and_resolve_from_path(path, &ctx).await?;
    values.merge(&mut test_plan.values, &mut ctx)?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work()?;

    info!("creating output directory");
    ctx.create_dir_all(out_dir)?;
    let config_dir = ctx.dir_containing(ctx.canonicalize_path(path)?);
    ctx.set_current_dir(config_dir)?;

    if test_plan.matrix.is_empty() {
        info!("executing test plan");
        return run_one(test_plan, out_dir, &ctx).await;
    }

    let n = test_plan.n_matrix_variants();

    for (mut i, tp) in test_plan.iter_matrix_variants().enumerate() {
        i += 1;
        ctx.set_values(&tp.values);
        let sub_dir = out_dir.join(format!("matrix_variant_{i}"));
        info!("creating output directory for matrix variant {i}/{n}");
        ctx.create_dir_all(&sub_dir)?;

        info!("executing test plan {i}/{n}");
        run_one(tp, &sub_dir, &ctx).await?;
    }

    Ok(())
}

async fn run_one(
    mut test_plan: TestPlanConfig,
    out_dir: &Path,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("templating environment setup");
    let mut values = take(&mut test_plan.values);
    test_plan.try_template_envrionment_setup(&values)?;

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
    builder.append(test_plan.try_template_envrionment_teardown(&values));
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

    info!("done");

    Ok(())
}
