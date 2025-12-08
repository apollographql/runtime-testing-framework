use crate::{
    cli::Variables,
    commands::{get_context, load_and_resolve_test_plan},
};
use anyhow::anyhow;
use rtf_config::{
    SourceDir,
    checks::{self, Check},
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::{Template, TemplateContext},
};
use std::{env::current_dir, path::PathBuf};
use tracing::info;

pub async fn template_test_plan_local(
    config_file_path: &str,
    variables: Variables,
    check: bool,
) -> anyhow::Result<()> {
    let ctx = get_context();
    let cwd = current_dir()?;

    info!("loading and resolving test plan");
    let test_plan = load_and_resolve_test_plan(config_file_path, &ctx).await?;

    template_test_plan_with_context(test_plan, variables, check, cwd, ctx).await
}

pub async fn template_test_plan_github(
    org_repo_path: String,
    git_ref: Option<String>,
    variables: Variables,
    check: bool,
) -> anyhow::Result<()> {
    let ctx = get_context();
    let cwd = current_dir()?;

    let (org, repo_and_path) = org_repo_path.split_once('/').ok_or(anyhow!(
        "invalid GitHub uri: \"{org_repo_path}\" - GitHub uri must be in format ORG/REPO/PATH"
    ))?;
    let (repo, path) = repo_and_path.split_once('/').ok_or(anyhow!(
        "invalid GitHub uri: \"{org_repo_path}\" - GitHub uri must be in format ORG/REPO/PATH"
    ))?;

    info!("fetching and resolving test plan from GitHub");
    let test_plan =
        TestPlanConfig::try_load_and_resolve_from_github(org, repo, path, git_ref, &ctx).await?;

    template_test_plan_with_context(test_plan, variables, check, cwd, ctx).await
}

async fn template_test_plan_with_context(
    mut test_plan: TestPlanConfig,
    variables: Variables,
    check: bool,
    cwd: PathBuf,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    let override_sources = variables.merge(&mut test_plan, &SourceDir::local(cwd), &mut ctx)?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work()?;

    let (_, variables) = test_plan.matrix.try_expand(&test_plan.variables)?.remove(0);
    let source = test_plan.sources.test_plan().clone();
    let template_ctx = TemplateContext::new(
        variables,
        source.clone(),
        override_sources,
        test_plan.sources.custom_providers(),
    );

    test_plan.try_template(&mut Vec::new(), &source, &template_ctx)?;

    if check {
        info!("checking test plan");
        let mut builder = checks::ErrorBuilder::from(
            test_plan
                .environment
                .setup
                .command
                .try_check(&mut vec!["setup".to_string()], &ctx),
        );
        builder.append(
            test_plan
                .scenario
                .command
                .try_check(&mut vec!["scenario".to_string()], &ctx),
        );
        builder.append(
            test_plan
                .environment
                .teardown
                .try_check(&mut vec!["teardown".to_string()], &ctx),
        );
        builder.into_result(())?;
    }

    println!("{}", test_plan.as_yaml_string_without_sources()?);

    Ok(())
}
