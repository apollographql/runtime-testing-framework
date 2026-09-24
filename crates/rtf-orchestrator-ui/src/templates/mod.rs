mod error;
mod execution;
mod index;
mod run;
mod test_plan_detail;
mod test_plans;
mod trigger;

pub use error::ErrorTemplate;
pub use execution::{ExecutionNotFoundTemplate, ExecutionTemplate};
pub use index::IndexTemplate;
pub use run::{RunNotFoundTemplate, RunTemplate};
pub use test_plan_detail::{TestPlanDetailTemplate, TestPlanNotFoundTemplate};
pub use test_plans::TestPlansTemplate;
pub use trigger::TriggerTemplate;

/// Panics on empty output: askama silently renders `""` for blocks nested inside control flow.
#[cfg(test)]
#[macro_export]
macro_rules! render_fragment {
    ($t:expr) => {{
        let body = $t.render().unwrap();
        assert!(
            !body.trim().is_empty(),
            "fragment rendered as an empty string"
        );

        body
    }};
}
