use crate::{
    cli::Variables,
    commands::{
        get_context, load_and_resolve_test_plan_from_github, load_and_resolve_test_plan_from_local,
    },
};
use rtf_config::{
    SourceDir,
    checks::Check,
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::{Template, TemplateContext},
};
use std::{env::current_dir, path::PathBuf};
use tracing::info;

pub async fn template_test_plan(
    test_plan_path: &str,
    check: bool,
    github: bool,
    git_ref: Option<String>,
    variables: Variables,
) -> anyhow::Result<()> {
    let mut ctx = get_context();
    let cwd = current_dir()?;

    info!("loading and resolving test plan");
    let test_plan = if github {
        load_and_resolve_test_plan_from_github(test_plan_path, git_ref, &ctx).await?
    } else {
        load_and_resolve_test_plan_from_local(test_plan_path, &ctx).await?
    };
    ctx.set_sources(test_plan.sources.clone());

    template_test_plan_with_context(test_plan, variables, check, cwd, ctx).await
}

async fn template_test_plan_with_context(
    mut test_plan: TestPlanConfig,
    variables: Variables,
    check: bool,
    cwd: PathBuf,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    let variable_sources = variables.merge(&mut test_plan, &SourceDir::local(cwd), &mut ctx)?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work(&variable_sources, &ctx)?;

    let (_, variables) = test_plan.matrix.try_expand(&test_plan.variables)?.remove(0);
    let source = test_plan.sources.test_plan().clone();
    let template_ctx = TemplateContext::new(
        variables,
        source.clone(),
        variable_sources,
        test_plan.sources.custom_providers(),
    );

    test_plan.try_template(&mut Vec::new(), &source, &template_ctx)?;

    if check {
        info!("checking test plan");
        test_plan.try_check(&mut Vec::new(), &ctx)?;
    }

    println!("{}", test_plan.as_yaml_string_without_sources()?);

    Ok(())
}
