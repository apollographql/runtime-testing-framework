//! Resolve file providers for an environment independently of executing it.
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
    formats::{
        DockerComposeEnvironment, EnvironmentConfig, EnvironmentExecution, ScriptEnvironment,
    },
    run::{OUTPUT_PATH, PROVIDER_DIR, RunProviders},
    templating::{Template, TemplateContext},
};
use std::{collections::HashMap, env::current_dir, path::Path};
use tracing::info;

const SETUP_ENV_FILE: &str = "setup.env";
const TEARDOWN_ENV_FILE: &str = "teardown.env";
const COMPOSE_FILES_LIST: &str = "compose-files.txt";

pub async fn resolve_environment(
    environment_path: &str,
    variables: Variables,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (mut ctx, out_dir) = get_context_and_check_outdir(out_dir)?;
    let cwd = current_dir()?;
    let cwd_source = SourceDir::local(cwd);

    info!("loading environment");
    let (source, mut environment) =
        load_config::<EnvironmentConfig>(environment_path, "environment", &ctx).await?;

    // Custom providers are not supported by resolve environment
    if !environment.custom_providers.is_empty() {
        bail!("custom_providers are not supported by resolve environment");
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

    info!("templating environment");
    environment.try_template(&mut Vec::new(), &source, &template_ctx)?;

    info!("running static checks");
    environment.try_check(&mut vec!["environment".to_string()], &ctx)?;

    info!("creating output directory");
    ctx.create_dir_all(&out_dir)?;
    let out_dir = ctx.canonicalize_path(&out_dir)?;
    ctx.set_output_path(&out_dir);

    match &environment.execution {
        EnvironmentExecution::Script(script) => {
            resolve_script_environment(script, &out_dir, &mut ctx).await?;
        }
        EnvironmentExecution::DockerCompose(compose) => {
            resolve_docker_compose_environment(&environment.name, compose, &out_dir, &mut ctx)
                .await?;
        }
    }

    info!("done");

    Ok(())
}

async fn resolve_script_environment(
    env: &ScriptEnvironment,
    out_dir: &Path,
    ctx: &mut impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("resolving setup providers");
    let setup_dir = out_dir.join("setup");
    ctx.create_dir_all(&setup_dir)?;
    let setup_providers_dir = setup_dir.join(PROVIDER_DIR);
    env.setup.run_providers(&setup_providers_dir, ctx).await?;

    info!("writing setup.env");
    let setup_output_path = setup_dir.join(OUTPUT_PATH);
    let setup_vars = env
        .setup
        .all_env_vars(&setup_dir, &setup_output_path, ctx)?;
    let setup_env_content = generate_env_file(setup_vars);
    ctx.write(setup_dir.join(SETUP_ENV_FILE), setup_env_content)?;

    info!("resolving teardown providers");
    let teardown_dir = out_dir.join("teardown");
    ctx.create_dir_all(&teardown_dir)?;
    let teardown_providers_dir = teardown_dir.join(PROVIDER_DIR);
    env.teardown
        .run_providers(&teardown_providers_dir, ctx)
        .await?;

    info!("writing teardown.env");
    let teardown_output_path = teardown_dir.join(OUTPUT_PATH);
    let teardown_vars = env
        .teardown
        .all_env_vars(&teardown_dir, &teardown_output_path, ctx)?;
    let teardown_env_content = generate_env_file(teardown_vars);
    ctx.write(teardown_dir.join(TEARDOWN_ENV_FILE), teardown_env_content)?;

    Ok(())
}

async fn resolve_docker_compose_environment(
    name: &str,
    env: &DockerComposeEnvironment,
    out_dir: &Path,
    ctx: &mut impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("resolving setup providers");
    let setup_dir = out_dir.join("setup");
    ctx.create_dir_all(&setup_dir)?;
    let setup_providers_dir = setup_dir.join(PROVIDER_DIR);
    env.run_providers(&setup_providers_dir, ctx).await?;

    // Write compose file paths to a file (one path per line)
    info!("writing compose-files.txt");
    let compose_paths: Vec<String> = env
        .compose_file_paths(ctx)?
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let compose_files_path = setup_dir.join(COMPOSE_FILES_LIST);
    let compose_files_content = compose_paths.join("\n");
    ctx.write(&compose_files_path, compose_files_content)?;

    info!("writing setup.env");
    let setup_output_path = setup_dir.join(OUTPUT_PATH);
    let mut setup_vars = env.all_env_vars(&setup_dir, &setup_output_path, ctx)?;
    setup_vars.insert(
        "COMPOSE_FILES".to_string(),
        compose_files_path.to_string_lossy().to_string(),
    );

    let setup_env_content = generate_env_file(setup_vars);
    ctx.write(setup_dir.join(SETUP_ENV_FILE), setup_env_content)?;

    info!("writing teardown.env");
    let teardown_dir = out_dir.join("teardown");
    ctx.create_dir_all(&teardown_dir)?;
    // Create empty providers dir for consistency
    ctx.create_dir_all(teardown_dir.join(PROVIDER_DIR))?;

    let project_name = env.project_name.as_deref().unwrap_or(name);
    let teardown_vars =
        HashMap::from([("COMPOSE_PROJECT_NAME".to_string(), project_name.to_string())]);
    let teardown_env_content = generate_env_file(teardown_vars);
    ctx.write(teardown_dir.join(TEARDOWN_ENV_FILE), teardown_env_content)?;

    Ok(())
}
