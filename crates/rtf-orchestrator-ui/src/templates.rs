use crate::view::{ExecutionDetailView, RunView};
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
#[template(path = "run_not_found.html")]
pub struct RunNotFoundTemplate {
    pub id: String,
}

/// The execution detail page: status-history timeline and metadata.
#[derive(Template)]
#[template(path = "execution.html")]
pub struct ExecutionTemplate {
    pub execution: ExecutionDetailView,
}

/// Shown when no execution with the requested id exists.
#[derive(Template)]
#[template(path = "execution_not_found.html")]
pub struct ExecutionNotFoundTemplate {
    pub execution_id: String,
}

/// Shown when fetching the run from the orchestrator fails unexpectedly.
#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub message: String,
}
