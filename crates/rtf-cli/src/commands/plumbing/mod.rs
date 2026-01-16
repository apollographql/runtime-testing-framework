//! Lower level commands for running individual pieces of functionality from the framework.

mod custom_provider;
mod expand_matrix;
mod inline;
mod template;

pub use custom_provider::{run_custom_provider, template_custom_provider, test_custom_provider};
pub use expand_matrix::expand_test_plan_matrix;
pub use inline::{InlineMode, inline_test_plan};
pub use template::template_test_plan;
