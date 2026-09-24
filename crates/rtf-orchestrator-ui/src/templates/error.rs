use askama::Template;

/// Shown when fetching the run from the orchestrator fails unexpectedly.
#[derive(Debug, Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub message: String,
}
