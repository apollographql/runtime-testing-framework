use crate::view::RunView;
use askama::Template;

/// The landing page: a form to enter a test-run id.
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate;

/// The run status page: overall status banner plus the executions table.
#[derive(Template)]
#[template(path = "run.html")]
pub struct RunTemplate {
    pub run: RunView,
}

/// Shown when a well-formed run id does not correspond to any known run.
#[derive(Template)]
#[template(path = "not_found.html")]
pub struct RunNotFoundTemplate {
    pub id: String,
}

/// Shown when fetching the run from the orchestrator fails unexpectedly.
#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub message: String,
}
