use crate::view::ExecutionDetailView;
use askama::Template;

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
