use crate::commands::{
    bail_on_template_and_check_errors, get_context, load_and_resolve_test_plan_from_github,
    load_and_resolve_test_plan_from_local,
};
use rtf_config::{
    StableSource,
    context::ResolutionContext,
    formats::{PrepareOnlyTestPlanConfig, Sources},
    templating::{Template, TemplateContext},
};
use rtf_core::variables::Variables;
use tracing::info;

pub async fn template_test_plan(
    test_plan_path: &str,
    check: bool,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<()> {
    let ctx = get_context();

    info!("loading and resolving test plan");
    let (test_plan, sources) = if github {
        load_and_resolve_test_plan_from_github(test_plan_path, git_ref, &ctx).await?
    } else {
        load_and_resolve_test_plan_from_local(test_plan_path, &ctx).await?
    };
    template_test_plan_with_context(test_plan, sources, variables, check, ctx).await
}

async fn template_test_plan_with_context(
    mut test_plan: PrepareOnlyTestPlanConfig,
    sources: Sources,
    variables: Variables,
    check: bool,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
    ctx.set_sources(sources.with_variables_file(vars_file_src));

    info!("checking if templating will work");
    test_plan.check_templating_will_work(&variable_sources, &ctx)?;

    info!("templating all test plan variants");
    bail_on_template_and_check_errors(test_plan.try_template_and_check_variants(
        &variable_sources,
        check,
        &ctx,
    )?)?;

    let (_, variables) = test_plan.matrix.try_expand(&test_plan.variables)?.remove(0);
    let template_ctx = TemplateContext::new(
        variables,
        variable_sources,
        ctx.custom_provider_definitions(),
    );

    test_plan.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)?;

    println!("{}", yaml_serde::to_string(&test_plan)?);

    Ok(())
}
