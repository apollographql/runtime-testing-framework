use crate::commands::get_context_and_outdir;
use rtf_config::{
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating,
    validation::{self, Validate},
};
use std::{mem::take, path::Path};
use tracing::info;

pub async fn validate_and_run_test_plan(
    config_file_path: &str,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_outdir(out_dir)?;
    validate_and_run_test_plan_with_context(config_file_path, &out_dir, ctx).await
}

async fn validate_and_run_test_plan_with_context(
    path: &str,
    out_dir: &Path,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let mut test_plan = TestPlanConfig::try_load_and_resolve_from_path(path, &ctx).await?;

    info!("checking if templating will work");
    test_plan.validate_templating_will_work()?;

    info!("creating output directory");
    ctx.create_dir_all(out_dir)?;
    let config_dir = ctx.dir_containing(ctx.canonicalize_path(path)?);
    ctx.set_current_dir(config_dir)?;

    info!("resolving environment setup");
    let mut values = take(&mut test_plan.values);
    test_plan.try_resolve_envrionment_setup(&values)?;

    info!("validating environment setup");
    test_plan.environment.setup.command.try_validate(
        &mut Vec::new(),
        test_plan.sources.environment(),
        &ctx,
    )?;

    info!("executing environment setup");
    let setup_provides = test_plan.run_environment_setup(out_dir, &ctx).await?;
    values.extend(setup_provides);

    info!("resolving scenario and environment teardown commands");
    let mut builder = templating::ErrorBuilder::from(test_plan.try_resolve_scenario(&values));
    builder.append(test_plan.try_resolve_envrionment_teardown(&values));
    builder.into_result(())?;

    info!("validating scenario and environment teardown commands");
    let mut builder = validation::ErrorBuilder::from(test_plan.scenario.command.try_validate(
        &mut Vec::new(),
        test_plan.sources.scenario(),
        &ctx,
    ));
    builder.append(test_plan.environment.teardown.try_validate(
        &mut Vec::new(),
        test_plan.sources.environment(),
        &ctx,
    ));
    builder.into_result(())?;

    info!("executing scenario");
    test_plan.run_scenario(out_dir, &ctx).await?;

    info!("executing environment teardown");
    test_plan.run_environment_teardown(out_dir, &ctx).await?;

    info!("done");

    Ok(())
}
