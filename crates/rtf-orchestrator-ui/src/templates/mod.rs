/// Renders a block fragment, failing on empty output: askama silently renders `""` for a block
/// nested inside control flow (`match`, `if let`) rather than failing to compile, so an empty
/// snapshot would otherwise be easy to accept by mistake.
#[cfg(test)]
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
