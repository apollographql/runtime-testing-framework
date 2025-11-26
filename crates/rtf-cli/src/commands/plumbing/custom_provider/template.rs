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
    Source,
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
    let cwd_source = Source::local(cwd.join("cli"));

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
        override_sources,
        Default::default(),
    );

    definition.try_template(&mut Vec::new(), &source, &template_ctx)?;

    if check {
        definition
            .command
            .try_check(&mut vec!["custom_provider".to_string()], &ctx)?;
    }

    println!("{}", serde_yaml::to_string(&definition)?);

    Ok(())
}
