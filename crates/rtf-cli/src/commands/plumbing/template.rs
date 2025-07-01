use crate::commands::{get_context, parse_values};
use rtf_config::{
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::{Scalar, Template},
    validation::{self, Validate},
};
use std::{collections::HashMap, mem::take};
use tracing::info;

pub async fn template_test_plan(
    config_file_path: &str,
    raw_values: Option<&str>,
    validate: bool,
) -> anyhow::Result<()> {
    let ctx = get_context();
    let values = match raw_values {
        Some(s) => Some(parse_values(s)?),
        None => None,
    };

    template_test_plan_with_context(config_file_path, values, validate, ctx).await
}

async fn template_test_plan_with_context(
    path: &str,
    values: Option<HashMap<String, Scalar>>,
    validate: bool,
    ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let mut test_plan = TestPlanConfig::try_load_and_resolve_from_path(path, &ctx).await?;

    info!("checking if templating will work");
    test_plan.validate_templating_will_work()?;

    info!("applying values");
    let mut values = values.unwrap_or_default();
    values.extend(take(&mut test_plan.expanded_matrix_values()[0]));
    test_plan.try_template(&mut Vec::new(), &values)?;

    if validate {
        info!("validating test plan");
        let mut builder =
            validation::ErrorBuilder::from(test_plan.environment.setup.command.try_validate(
                &mut Vec::new(),
                test_plan.sources.environment(),
                &ctx,
            ));
        builder.append(test_plan.scenario.command.try_validate(
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
    }

    println!("{}", serde_yaml::to_string(&test_plan)?);

    Ok(())
}
