//! Run a custom provider independently of a test plan
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{
        get_context_and_check_outdir, load_config,
        plumbing::{
            custom_provider::{RESOLVED_PROVIDER_PATH, VARIABLES_PATH},
            parse_cli_variables,
        },
    },
};
use rtf_config::{
    SourceDir,
    checks::Check,
    context::ResolutionContext,
    formats::CustomProviderDefinition,
    run::{Execute, OUTPUT_PATH, PROVIDER_DIR},
    templating::{Template, TemplateContext},
};
use std::env::current_dir;
use tracing::info;

pub async fn run_custom_provider(
    definition_path: &str,
    variables: Variables,
    out_dir: &str,
    force: bool,
) -> anyhow::Result<()> {
    let (mut ctx, out_dir) = get_context_and_check_outdir(out_dir, force)?;
    let cwd = current_dir()?;
    let cwd_source = SourceDir::local(cwd);

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
    } = parse_cli_variables(variables, &cwd_source, &ctx)?;

    let template_ctx = TemplateContext::new(
        variables,
        source.clone(),
        variable_sources.clone(),
        Default::default(),
    );

    definition.validate_variables(template_ctx.variables(), Some(&variable_sources))?;

    definition.try_template(&mut Vec::new(), &source, &template_ctx)?;
    definition
        .command
        .try_check(&mut vec!["custom_provider".to_string()], &ctx)?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    let out_dir = ctx.canonicalize_path(&out_dir)?;
    ctx.set_output_path(&out_dir);

    info!("executing custom provider");
    definition
        .command
        .run_providers_and_execute(
            "unknown",
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
        definition.as_yaml_string_without_sources()?,
    )?;

    info!("done");

    Ok(())
}
