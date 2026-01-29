//! Commands for checking and running custom provider definitions independently.
use anyhow::Context;
use rtf_config::{SourceDir, context::ResolutionContext, formats::CustomProviderDefinition};

mod run;
mod template;
mod test;

pub use run::run_custom_provider;
pub use template::template_custom_provider;
pub use test::test_custom_provider;

const VARIABLES_PATH: &str = "provider-variables.json";
const RESOLVED_PROVIDER_PATH: &str = "resolved-provider.yaml";

async fn load_definition(
    path: &str,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<(SourceDir, CustomProviderDefinition)> {
    let abs_path = ctx
        .canonicalize_path(path)
        .with_context(|| format!("Unable to resolve path: {path}"))?;
    let content = ctx
        .read_path_to_string(&abs_path)
        .with_context(|| format!("Unable to read custom provider definition from {path}"))?;
    let source = SourceDir::local(
        abs_path
            .parent()
            .expect("we just read the file so it has a parent"),
    );

    let definition: CustomProviderDefinition = serde_yaml::from_str(&content)
        .with_context(|| "Unable to parse custom provider definition yaml")?;

    Ok((source, definition))
}
