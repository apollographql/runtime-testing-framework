//! Run a custom provider independently of a test plan
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{
        get_context_and_check_outdir,
        plumbing::custom_provider::{
            RESOLVED_PROVIDER_PATH, VARIABLES_PATH, load_definition, parse_cli_variables,
        },
    },
};
use rtf_config::{
    Source,
    checks::Check,
    context::ResolutionContext,
    providers::command::{OUTPUT_PATH, PROVIDER_DIR},
    templating::{Template, TemplateContext},
};
use std::env::current_dir;
use tracing::info;

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
        serde_yaml::to_string(&definition)?,
    )?;

    info!("done");

    Ok(())
}
