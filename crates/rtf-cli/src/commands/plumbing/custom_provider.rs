//! Commands for checking and running custom provider definitions independently.

use crate::cli::Variables;

/// Check a custom provider definition without executing it.
///
/// This validates that the definition is well-formed, templates correctly with the provided
/// variables, and passes static analysis checks.
pub async fn check_custom_provider(
    _definition_path: &str,
    _variables: Variables,
) -> anyhow::Result<()> {
    todo!("implement check_custom_provider")
}

/// Execute a custom provider definition.
///
/// This loads the definition, templates it with the provided variables, runs static checks,
/// and then executes the provider command, writing output to the specified directory.
pub async fn run_custom_provider(
    _definition_path: &str,
    _variables: Variables,
    _out_dir: &str,
) -> anyhow::Result<()> {
    todo!("implement run_custom_provider")
}
