//! Template a custom provider independently of a test plan
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{get_context, load_config, plumbing::parse_cli_variables},
};
use rtf_config::{
    StableSource,
    checks::Check,
    formats::CustomProviderDefinition,
    templating::{Template, TemplateContext},
};
use tracing::info;

pub async fn template_custom_provider(
    definition_path: &str,
    variables: Variables,
    check: bool,
) -> anyhow::Result<()> {
    let mut ctx = get_context();

    info!("loading custom provider definition");
    let (source, mut definition) = load_config::<CustomProviderDefinition>(
        definition_path,
        "custom provider definition",
        &ctx,
    )
    .await?;

    let ParsedVariables {
        variables,
        variable_sources,
        ..
    } = parse_cli_variables(variables, source, &mut ctx)?;

    let template_ctx = TemplateContext::new(
        variables,
        StableSource::Cli,
        variable_sources.clone(),
        Default::default(),
    );

    definition.validate_variables(template_ctx.variables(), Some(&variable_sources))?;

    definition.try_template(&mut Vec::new(), &StableSource::Cli, &template_ctx)?;

    if check {
        definition
            .command
            .try_check(&mut vec!["custom_provider".to_string()], &ctx)?;
    }

    println!("{}", definition.as_yaml_string()?);

    Ok(())
}
