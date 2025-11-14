use crate::{
    cli::Variables,
    commands::{get_context, load_and_resolve_test_plan},
};
use rtf_config::{
    Source,
    checks::{self, Check},
    context::ResolutionContext,
    templating::{Template, TemplateVariables},
};
use std::{env::current_dir, path::PathBuf};
use tracing::info;

pub async fn template_test_plan(
    config_file_path: &str,
    variables: Variables,
    check: bool,
) -> anyhow::Result<()> {
    let ctx = get_context();
    let cwd = current_dir()?;

    template_test_plan_with_context(config_file_path, variables, check, cwd, ctx).await
}

async fn template_test_plan_with_context(
    path: &str,
    variables: Variables,
    check: bool,
    cwd: PathBuf,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let mut test_plan = load_and_resolve_test_plan(path, &ctx).await?;
    let override_sources =
        variables.merge(&mut test_plan, &Source::local(cwd.join("cli")), &mut ctx)?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work()?;

    let (_, variables) = test_plan.matrix.try_expand(&test_plan.variables)?.remove(0);
    let source = test_plan.sources.test_plan().clone();
    let template_variables = TemplateVariables::new(variables, source.clone(), override_sources);

    test_plan.try_template(&mut Vec::new(), &source, &template_variables)?;

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

    println!("{}", serde_yaml::to_string(&test_plan)?);

    Ok(())
}
