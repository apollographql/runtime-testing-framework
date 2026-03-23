//! Lower level commands for running individual pieces of functionality from the framework.
use crate::{ParsedVariables, cli::Variables};
use anyhow::anyhow;
use rtf_config::{SourceDir, context::ResolutionContext};

mod completion;
mod custom_provider;
mod expand_matrix;
mod inline;
mod json_schemas;
mod rep;
mod resolve;
mod template;

pub use completion::generate_shell_completions;
pub use custom_provider::{run_custom_provider, template_custom_provider, test_custom_provider};
pub use expand_matrix::expand_test_plan_matrix;
pub use inline::inline_test_plan;
pub use json_schemas::generate_json_schema;
pub use rep::{prepare_rep_trigger_payload, write_rep_trigger_payload_to_stdout};
pub use resolve::{resolve_environment, resolve_scenario};
pub use template::template_test_plan;

fn parse_cli_variables(
    variables: Variables,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<(ParsedVariables, Option<SourceDir>)> {
    let (parsed, vars_file_src) = variables.parse(ctx)?;

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

    Ok((parsed, vars_file_src))
}
