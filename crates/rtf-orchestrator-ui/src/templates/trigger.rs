use askama::Template;

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
