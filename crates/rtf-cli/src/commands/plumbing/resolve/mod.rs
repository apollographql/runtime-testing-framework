//! Commands for resolving file providers independently of executing commands.
use anyhow::Context;
use rtf_config::{
    SourceDir,
    context::ResolutionContext,
    formats::{EnvironmentConfig, ScenarioConfig},
};
use std::collections::HashMap;

pub mod environment;
pub mod scenario;

pub use environment::resolve_environment;
pub use scenario::resolve_scenario;

/// Generate an env file from a map of environment variables.
fn generate_env_file(env_vars: HashMap<String, String>) -> String {
    let mut sorted_vars: Vec<_> = env_vars.into_iter().collect();
    sorted_vars.sort_by(|(a, _), (b, _)| a.cmp(b));

    sorted_vars
        .into_iter()
        .fold(String::new(), |mut acc, (key, value)| {
            acc.push_str(&format!("export {key}=\"{value}\"\n"));
            acc
        })
}

async fn load_scenario(
    path: &str,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<(SourceDir, ScenarioConfig)> {
    let abs_path = ctx
        .canonicalize_path(path)
        .with_context(|| format!("Unable to resolve path: {path}"))?;
    let content = ctx
        .read_path_to_string(&abs_path)
        .with_context(|| format!("Unable to read scenario from {path}"))?;
    let source = SourceDir::local(
        abs_path
            .parent()
            .expect("we just read the file so it has a parent"),
    );

    let scenario: ScenarioConfig =
        serde_yaml::from_str(&content).with_context(|| "Unable to parse scenario yaml")?;

    Ok((source, scenario))
}

async fn load_environment(
    path: &str,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<(SourceDir, EnvironmentConfig)> {
    let abs_path = ctx
        .canonicalize_path(path)
        .with_context(|| format!("Unable to resolve path: {path}"))?;
    let content = ctx
        .read_path_to_string(&abs_path)
        .with_context(|| format!("Unable to read environment from {path}"))?;
    let source = SourceDir::local(
        abs_path
            .parent()
            .expect("we just read the file so it has a parent"),
    );

    let environment: EnvironmentConfig =
        serde_yaml::from_str(&content).with_context(|| "Unable to parse environment yaml")?;

    Ok((source, environment))
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
    fn generate_env_file_formats_correctly(vars: &[(&str, &str)], expected: &str) {
        let env_vars = HashMap::from_iter(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let result = generate_env_file(env_vars);

        assert_eq!(result, expected);
    }
}
