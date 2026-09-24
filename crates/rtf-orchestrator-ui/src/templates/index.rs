use crate::view::RunListView;
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
