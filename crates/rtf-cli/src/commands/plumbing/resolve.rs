use crate::commands::{get_context, parse_values};
use rtf_config::{
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::{Scalar, Template},
};
use std::{collections::HashMap, mem::take};
use tracing::info;

pub async fn resolve_test_plan(
    config_file_path: &str,
    raw_values: Option<&str>,
) -> anyhow::Result<()> {
    let ctx = get_context();
    let values = match raw_values {
        Some(s) => Some(parse_values(s)?),
        None => None,
    };

    resolve_test_plan_with_context(config_file_path, values, ctx).await
}

async fn resolve_test_plan_with_context(
    path: &str,
    values: Option<HashMap<String, Scalar>>,
    ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let mut test_plan = TestPlanConfig::try_load_and_resolve_from_path(path, &ctx).await?;

    info!("checking if templating will work");
    test_plan.validate_templating_will_work()?;

    info!("applying values");
    let mut values = values.unwrap_or_default();
    values.extend(take(&mut test_plan.values));
    test_plan.try_resolve(&mut Vec::new(), &values)?;

    let s = serde_yaml::to_string(&test_plan)?;
    println!("{s}");

    Ok(())
}
