use crate::commands::{get_context, load_and_resolve_test_plan};
use rtf_config::context::ResolutionContext;
use serde_json::json;
use tracing::info;

pub async fn expand_test_plan_matrix(config_file_path: &str, compact: bool) -> anyhow::Result<()> {
    let ctx = get_context();

    expand_test_plan_matrix_with_context(config_file_path, compact, ctx).await
}

async fn expand_test_plan_matrix_with_context(
    path: &str,
    compact: bool,
    ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let test_plan = load_and_resolve_test_plan(path, &ctx).await?;

    info!("expanding test plan matrix");
    let expanded: Vec<_> = test_plan
        .matrix
        .try_expand(&test_plan.variables)?
        .iter()
        .map(|(name, values)| json!({"name": name, "values": values}))
        .collect();

    let variants = json!({"variants": expanded});

    if compact {
        println!("{}", serde_json::to_string(&variants)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&variants)?);
    }

    Ok(())
}
