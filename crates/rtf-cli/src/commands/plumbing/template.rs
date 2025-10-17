use crate::{cli::Values, commands::get_context};
use rtf_config::{
    checks::{self, Check},
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::Template,
};
use tracing::info;

pub async fn template_test_plan(
    config_file_path: &str,
    values: Values,
    check: bool,
) -> anyhow::Result<()> {
    let ctx = get_context();

    template_test_plan_with_context(config_file_path, values, check, ctx).await
}

async fn template_test_plan_with_context(
    path: &str,
    values: Values,
    check: bool,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let mut test_plan = TestPlanConfig::try_load_and_resolve_from_path(path, &ctx).await?;
    values.merge(
        &mut test_plan.values,
        &mut test_plan.matrix.dimensions,
        &mut ctx,
    )?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work()?;

    let (_, values) = &test_plan.matrix.try_expand(&test_plan.values)?[0];
    test_plan.try_template(&mut Vec::new(), values)?;

    if check {
        info!("checking test plan");
        let mut builder =
            checks::ErrorBuilder::from(test_plan.environment.setup.command.try_check(
                &mut Vec::new(),
                test_plan.sources.environment(),
                &ctx,
            ));
        builder.append(test_plan.scenario.command.try_check(
            &mut Vec::new(),
            test_plan.sources.scenario(),
            &ctx,
        ));
        builder.append(test_plan.environment.teardown.try_check(
            &mut Vec::new(),
            test_plan.sources.environment(),
            &ctx,
        ));
        builder.into_result(())?;
    }

    println!("{}", serde_yaml::to_string(&test_plan)?);

    Ok(())
}
