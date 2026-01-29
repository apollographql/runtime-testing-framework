//! Commands for resolving file providers independently of executing commands.
use anyhow::Context;
use rtf_config::{SourceDir, context::ResolutionContext, formats::ScenarioConfig};

pub mod scenario;

pub use scenario::resolve_scenario;

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
