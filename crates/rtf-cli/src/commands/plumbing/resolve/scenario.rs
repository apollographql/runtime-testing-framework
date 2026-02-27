//! Resolve file providers for a scenario independently of executing it.
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{
        get_context_and_check_outdir, load_config,
        plumbing::{parse_cli_variables, resolve::generate_env_file},
    },
};
use anyhow::bail;
use rtf_config::{
    SourceDir,
    checks::Check,
    context::ResolutionContext,
    formats::ScenarioConfig,
    run::{OUTPUT_PATH, PROVIDER_DIR, RunProviders},
    templating::{Template, TemplateContext},
};
use std::env::current_dir;
use tracing::info;

const SCENARIO_ENV_FILE: &str = "scenario.env";

pub async fn resolve_scenario(
    scenario_path: &str,
    variables: Variables,
    out_dir: &str,
    force: bool,
) -> anyhow::Result<()> {
    let (mut ctx, out_dir) = get_context_and_check_outdir(out_dir, force)?;
    let cwd = current_dir()?;
    let cwd_source = SourceDir::local(cwd);

    info!("loading scenario");
    let (source, mut scenario) =
        load_config::<ScenarioConfig>(scenario_path, "scenario", &ctx).await?;

    // Custom providers are not supported by resolve scenario
    if !scenario.custom_providers.is_empty() {
        bail!("custom_providers are not supported by resolve scenario");
    }

    let ParsedVariables {
        variables,
        variable_sources,
        ..
    } = parse_cli_variables(variables, &cwd_source, &ctx)?;

    let template_ctx = TemplateContext::new(
        variables,
        source.clone(),
        variable_sources,
        Default::default(),
    );

    info!("templating scenario");
    scenario.try_template(&mut Vec::new(), &source, &template_ctx)?;

    info!("running static checks");
    scenario
        .command
        .try_check(&mut vec!["scenario".to_string()], &ctx)?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    let out_dir = ctx.canonicalize_path(&out_dir)?;
    ctx.set_output_path(&out_dir);

    info!("resolving file providers");
    let providers_dir = out_dir.join(PROVIDER_DIR);
    scenario
        .command
        .run_providers(&providers_dir, &mut ctx)
        .await?;

    info!("writing scenario.env");
    let output_path = out_dir.join(OUTPUT_PATH);
    let env_vars = scenario
        .command
        .all_env_vars(&out_dir, &output_path, &ctx)?;
    let env_content = generate_env_file(env_vars);
    ctx.write(out_dir.join(SCENARIO_ENV_FILE), env_content)?;

    info!("done");

    Ok(())
}
