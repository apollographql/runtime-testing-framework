//! Template a custom provider independently of a test plan
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{get_context, load_config, plumbing::parse_cli_variables},
};
use rtf_config::{
    CustomProviderSection, SourceDir, StableSource,
    checks::Check,
    context::ResolutionContext,
    formats::{CustomProviderDefinition, Sources},
    templating::{Template, TemplateContext},
};
use std::env::current_dir;
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

    let (
        ParsedVariables {
            variables,
            variable_sources,
            ..
        },
        vars_file_src,
    ) = parse_cli_variables(variables, &ctx)?;

    let stable_src =
        StableSource::CustomProvider(definition.name.clone(), CustomProviderSection::TestPlan);
    ctx.set_sources(
        Sources::default()
            .with_custom_provider_source(definition.name.clone(), source)
            .with_cli(SourceDir::local(current_dir()?))
            .with_variables_file(vars_file_src),
    );

    let template_ctx =
        TemplateContext::new(variables, variable_sources.clone(), Default::default());

    definition.validate_variables(template_ctx.variables(), Some(&variable_sources))?;

    definition.try_template(&mut Vec::new(), &stable_src, &template_ctx)?;

    if check {
        definition
            .command
            .try_check(&mut vec!["custom_provider".to_string()], &ctx)?;
    }

    println!("{}", serde_yaml::to_string(&definition)?);

    Ok(())
}
