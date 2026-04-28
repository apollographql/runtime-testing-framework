use crate::commands::{
    get_context, load_and_resolve_test_plan_from_github, load_and_resolve_test_plan_from_local,
};
use rtf_config::{
    StableSource,
    checks::Check,
    context::ResolutionContext,
    formats::{Sources, TestPlanConfig},
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
    mut test_plan: TestPlanConfig,
    sources: Sources,
    variables: Variables,
    check: bool,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
    ctx.set_sources(sources.with_variables_file(vars_file_src));

    info!("checking if templating will work");
    test_plan.check_templating_will_work(&variable_sources, &ctx)?;

    let (_, variables) = test_plan.matrix.try_expand(&test_plan.variables)?.remove(0);
    let template_ctx = TemplateContext::new(
        variables,
        variable_sources,
        ctx.custom_provider_definitions(),
    );

    test_plan.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)?;

    if check {
        info!("checking test plan");
        test_plan.try_check(&mut Vec::new(), &ctx)?;
    }

    println!("{}", serde_yaml::to_string(&test_plan)?);

    Ok(())
}
