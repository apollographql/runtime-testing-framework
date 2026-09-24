use crate::view::RunView;
use askama::Template;

/// The run status page: overall status banner plus the executions table.
#[derive(Template)]
#[template(path = "run.html")]
pub struct RunTemplate {
    pub run: RunView,
}

/// Shown when a well-formed run id does not correspond to any known run.
#[derive(Template)]
#[template(path = "run_not_found.html")]
pub struct RunNotFoundTemplate {
    pub id: String,
}
