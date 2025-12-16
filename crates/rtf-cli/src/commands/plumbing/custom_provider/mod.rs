//! Commands for checking and running custom provider definitions independently.
use crate::{ParsedVariables, cli::Variables};
use anyhow::{Context, anyhow};
use rtf_config::{
    SourceDir,
    context::ResolutionContext,
    formats::CustomProviderDefinition,
    templating::{self, TemplateContext},
};
use std::collections::HashMap;

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

fn parse_cli_variables(
    variables: Variables,
    cwd_source: &SourceDir,
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

/// Validate variable definitions and values before templating
fn validate_variable_definitions(
    definition: &CustomProviderDefinition,
    override_sources: &HashMap<String, SourceDir>,
    template_ctx: &TemplateContext,
) -> templating::Result<()> {
    let mut errs = templating::ErrorBuilder::new();

    for vd in definition.variable_definitions.iter() {
        vd.validate(
            &["variable_definitions".to_string(), vd.name.to_string()],
            &mut errs,
        );

        // Validate CLI variable values against allowed_values
        if let Some(value) = template_ctx.variables().get(&vd.name) {
            let source_desc = if override_sources.contains_key(&vd.name) {
                "CLI variable"
            } else {
                "variable"
            };
            vd.validate_value(
                value,
                source_desc,
                &["variables".to_string(), vd.name.to_string()],
                &mut errs,
            );
        }
    }

    errs.into_result(())
}
