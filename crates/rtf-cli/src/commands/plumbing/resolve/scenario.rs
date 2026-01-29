//! Resolve file providers for a scenario independently of executing it.
use crate::{
    ParsedVariables,
    cli::Variables,
    commands::{
        get_context_and_check_outdir,
        plumbing::{parse_cli_variables, resolve::load_scenario},
    },
};
use anyhow::bail;
use rtf_config::{
    SourceDir,
    checks::Check,
    context::ResolutionContext,
    run::{OUTPUT_PATH, PROVIDER_DIR, RunProviders},
    templating::{Template, TemplateContext},
};
use std::{collections::HashMap, env::current_dir};
use tracing::info;

const SCENARIO_ENV_FILE: &str = "scenario.env";

pub async fn resolve_scenario(
    scenario_path: &str,
    variables: Variables,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (mut ctx, out_dir) = get_context_and_check_outdir(out_dir)?;
    let cwd = current_dir()?;
    let cwd_source = SourceDir::local(cwd);

    info!("loading scenario");
    let (source, mut scenario) = load_scenario(scenario_path, &ctx).await?;

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
    let env_content = generate_scenario_env(env_vars);
    ctx.write(out_dir.join(SCENARIO_ENV_FILE), env_content)?;

    info!("done");

    Ok(())
}

fn generate_scenario_env(env_vars: HashMap<String, String>) -> String {
    let mut sorted_vars: Vec<_> = env_vars.into_iter().collect();
    sorted_vars.sort_by(|(a, _), (b, _)| a.cmp(b));

    sorted_vars
        .into_iter()
        .fold(String::new(), |mut acc, (key, value)| {
            acc.push_str(&format!("export {key}=\"{value}\"\n"));
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    #[test_case(&[], ""; "empty")]
    #[test_case(&[("FOO", "bar")], "export FOO=\"bar\"\n"; "single variable")]
    #[test_case(&[("FOO", "hello world")], "export FOO=\"hello world\"\n"; "value with space")]
    #[test_case(&[("ZZZ", "last"), ("AAA", "first")], "export AAA=\"first\"\nexport ZZZ=\"last\"\n"; "multiple variables")]
    #[test]
    fn generate_scenario_env_formats_correctly(vars: &[(&str, &str)], expected: &str) {
        let env_vars = HashMap::from_iter(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let env_file_string = generate_scenario_env(env_vars);

        assert_eq!(env_file_string, expected);
    }
}
