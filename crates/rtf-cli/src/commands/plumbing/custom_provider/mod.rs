//! Commands for checking and running custom provider definitions independently.

mod run;
mod template;
mod test;

pub use run::run_custom_provider;
pub use template::template_custom_provider;
pub use test::test_custom_provider;

const VARIABLES_PATH: &str = "provider-variables.json";
const RESOLVED_PROVIDER_PATH: &str = "resolved-provider.yaml";
