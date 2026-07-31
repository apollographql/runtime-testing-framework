use crate::view::{ExecutionDetailView, RunListView, RunView};
use askama::Template;

/// The landing page: the run-id lookup form plus the recent-runs table.
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    /// Re-populates the "Initiated by" field after a search.
    pub initiated_by: String,
    /// Which `started_within` preset is selected ("", "hour", "day", "week", "month").
    pub started_within: String,
    pub list: Option<RunListView>,
    pub list_error: Option<String>,
}

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

/// The "trigger a run from GitHub" form. Fields are re-populated from the submission on a failed
/// attempt, alongside `error`, so the user doesn't have to retype everything to fix one mistake.
#[derive(Debug, Default, Template)]
#[template(path = "trigger.html")]
pub struct TriggerTemplate {
    pub org: String,
    pub repo: String,
    pub path: String,
    pub git_ref: String,
    /// A JSON object of scalar or array values, textarea-edited raw. An array value defines a
    /// matrix dimension rather than a single templating variable.
    pub variables: String,
    pub error: Option<String>,
}
