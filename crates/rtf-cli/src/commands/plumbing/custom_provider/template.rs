//! Template a custom provider independently of a test plan
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{
        get_context,
        plumbing::custom_provider::{load_definition, parse_cli_variables},
    },
};
use rtf_config::{
    SourceDir,
    checks::Check,
    templating::{Template, TemplateContext},
};
use std::env::current_dir;
use tracing::info;

pub async fn template_custom_provider(
    definition_path: &str,
    variables: Variables,
    check: bool,
) -> anyhow::Result<()> {
    let ctx = get_context();
    let cwd = current_dir()?;
    let cwd_source = SourceDir::local(cwd);

    info!("loading custom provider definition");
    let (source, mut definition) = load_definition(definition_path, &ctx).await?;

    let ParsedVariables {
        variables,
        override_sources,
        ..
    } = parse_cli_variables(variables, &cwd_source, &ctx)?;

    let template_ctx = TemplateContext::new(
        variables,
        source.clone(),
        override_sources.clone(),
        Default::default(),
    );

    definition.validate_variables(template_ctx.variables(), Some(&override_sources))?;

    definition.try_template(&mut Vec::new(), &source, &template_ctx)?;

    if check {
        definition
            .command
            .try_check(&mut vec!["custom_provider".to_string()], &ctx)?;
    }

    println!("{}", definition.as_yaml_string_without_sources()?);

    Ok(())
}
