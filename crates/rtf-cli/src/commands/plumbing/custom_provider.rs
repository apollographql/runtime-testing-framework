//! Commands for checking and running custom provider definitions independently.
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{get_context, get_context_and_check_outdir},
};
use anyhow::{Context, anyhow};
use rtf_config::{
    Source,
    checks::Check,
    context::ResolutionContext,
    formats::CustomProviderDefinition,
    providers::command::{OUTPUT_PATH, PROVIDER_DIR},
    templating::{Template, TemplateContext},
};
use std::env::current_dir;
use tracing::info;

const VARIABLES_PATH: &str = "provider-variables.json";
const RESOLVED_PROVIDER_PATH: &str = "resolved-provider.yaml";

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

pub async fn run_custom_provider(
    definition_path: &str,
    variables: Variables,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (mut ctx, out_dir) = get_context_and_check_outdir(out_dir)?;
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
    definition
        .command
        .try_check(&mut vec!["custom_provider".to_string()], &ctx)?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    let out_dir = ctx.canonicalize_path(&out_dir)?;

    if let Source::Local { abs_path } = &source {
        let definition_dir = ctx.dir_containing(abs_path);
        ctx.set_current_dir(definition_dir)?;
    }

    info!("executing custom provider");
    definition
        .command
        .run_providers_and_execute(
            &out_dir,
            out_dir.join(OUTPUT_PATH),
            out_dir.join(PROVIDER_DIR),
            &mut ctx,
        )
        .await?;

    info!("writing out resolved provider and variables");
    ctx.write(
        out_dir.join(VARIABLES_PATH),
        serde_json::to_string_pretty(template_ctx.variables())?,
    )?;
    ctx.write(
        out_dir.join(RESOLVED_PROVIDER_PATH),
        serde_yaml::to_string(&definition)?,
    )?;

    info!("done");

    Ok(())
}

async fn load_definition(
    path: &str,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<(Source, CustomProviderDefinition)> {
    let abs_path = ctx
        .canonicalize_path(path)
        .with_context(|| format!("Unable to resolve path: {path}"))?;
    let source = Source::local(&abs_path);
    let content = ctx
        .read_path_to_string(&abs_path)
        .with_context(|| format!("Unable to read custom provider definition from {path}"))?;

    let definition: CustomProviderDefinition = serde_yaml::from_str(&content)
        .with_context(|| "Unable to parse custom provider definition yaml")?;

    Ok((source, definition))
}

fn parse_cli_variables(
    variables: Variables,
    cwd_source: &Source,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<ParsedVariables> {
    let parsed = variables.parse(cwd_source, ctx)?;

    if !parsed.matrix_dimensions.is_empty() {
        let mut keys: Vec<_> = parsed
            .matrix_dimensions
            .keys()
            .map(|s| s.as_str())
            .collect();
        keys.sort_unstable();
        return Err(anyhow!(
            "Expected only scalar variables but found matrix dimensions for: {}",
            keys.join(", ")
        ));
    }

    Ok(parsed)
}
