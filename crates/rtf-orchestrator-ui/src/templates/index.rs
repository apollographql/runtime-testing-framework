use crate::view::RunListView;
use askama::Template;

/// The landing page: the run-id lookup form plus the recent-runs table.
#[derive(Template)]
#[template(path = "index.html", blocks = ["filters", "runs"])]
pub struct IndexTemplate {
    /// Re-populates the "Initiated by" field after a search.
    pub initiated_by: String,
    /// Which `started_within` preset is selected ("", "hour", "day", "week", "month").
    pub started_within: String,
    pub list: Option<RunListView>,
    pub list_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{orchestrator::mocks::sample_summary, render_fragment};
    use chrono::{TimeZone, Utc};
    use rtf_orchestrator_shared::{
        status::Status,
        summary::{TestRunListResponse, TestRunSummary},
    };
    use uuid::Uuid;

    fn index(initiated_by: &str, started_within: &str) -> IndexTemplate {
        let started_at = Utc.with_ymd_and_hms(2024, 3, 15, 12, 0, 0).unwrap();
        let response = TestRunListResponse {
            runs: vec![TestRunSummary {
                started_at,
                updated_at: started_at,
                ..sample_summary(Uuid::from_u128(1), Uuid::from_u128(2), Status::Running)
            }],
            total: 1,
        };

        IndexTemplate {
            initiated_by: initiated_by.to_owned(),
            started_within: started_within.to_owned(),
            list: Some(RunListView::new(
                response,
                20,
                0,
                initiated_by.to_owned(),
                started_within.to_owned(),
            )),
            list_error: None,
        }
    }

    #[test]
    fn filters_snapshot() {
        insta::assert_snapshot!(render_fragment!(index("", "").as_filters()));
    }

    #[test]
    fn filters_snapshot_repopulated() {
        insta::assert_snapshot!(render_fragment!(index("testuser", "week").as_filters()));
    }

    #[test]
    fn runs_snapshot() {
        insta::assert_snapshot!(render_fragment!(index("", "").as_runs()));
    }

    #[test]
    fn runs_snapshot_listing_failed() {
        let t = IndexTemplate {
            list: None,
            list_error: Some("Could not load recent runs from the orchestrator.".to_owned()),
            ..index("", "")
        };
        insta::assert_snapshot!(render_fragment!(t.as_runs()));
    }
}
