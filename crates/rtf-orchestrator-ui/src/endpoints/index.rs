use crate::{
    endpoints::{render_body, to_response},
    orchestrator::{self, Client, RunListFilter},
    templates::IndexTemplate,
    view::RunListView,
};
use axum::{
    extract::{Query, State},
    response::Response,
};
use chrono::{Duration, Utc};
use reqwest::StatusCode;
use rtf_orchestrator_shared::summary::TestRunListResponse;
use tracing::error;

const DEFAULT_LIMIT: i64 = 20;

/// Query params accepted by the home page.
#[derive(Debug, serde::Deserialize)]
pub struct IndexParams {
    initiated_by: Option<String>,
    started_within: Option<String>,
    offset: Option<i64>,
}

/// A coarse time-range preset rather than a raw datetime
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartedWithin {
    Hour,
    Day,
    Week,
    Month,
}

impl StartedWithin {
    /// Anything unrecognized is treated the same as absent rather than rejected.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "hour" => Some(Self::Hour),
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    fn as_duration(self) -> Duration {
        match self {
            Self::Hour => Duration::hours(1),
            Self::Day => Duration::days(1),
            Self::Week => Duration::days(7),
            Self::Month => Duration::days(30),
        }
    }
}

/// `GET /ui` — the run-id lookup box plus a filterable table of recent runs.
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Query(params): Query<IndexParams>,
) -> Response {
    let offset = params.offset.unwrap_or(0).max(0);
    // Trim so a whitespace-only box (stray leading/trailing space) doesn't silently filter on a
    // value nothing will ever match — an empty result then genuinely means "no filter applied."
    let initiated_by = params
        .initiated_by
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    // Kept as the raw string, not re-derived from `StartedWithin` — parsing it decides the actual
    // `started_after` filter below, but the display/pagination-link copy doesn't need a second,
    // separately-materialized value; an unrecognized value just passes through unchanged (the
    // `<select>` has no matching `<option>` for it, so it renders as "Any time" regardless).
    let started_within = params.started_within.unwrap_or_default();

    let filter = RunListFilter {
        initiated_by: initiated_by.clone(),
        started_after: StartedWithin::parse(&started_within).map(|w| Utc::now() - w.as_duration()),
        limit: DEFAULT_LIMIT,
        offset,
    };

    let list = orchestrator_client.list_runs(&filter).await;

    to_response(index_body(
        list,
        DEFAULT_LIMIT,
        offset,
        initiated_by,
        started_within,
    ))
}

/// Maps the result of listing recent runs to a rendered `(status, body)` pair. A fetch failure
/// still renders the page — with the run-id lookup box intact and an inline error in place of the
/// table — since that box doesn't depend on the list endpoint at all.
fn index_body(
    result: Result<TestRunListResponse, orchestrator::Error>,
    limit: i64,
    offset: i64,
    initiated_by: String,
    started_within: String,
) -> (StatusCode, String) {
    match result {
        Ok(response) => render_body(
            StatusCode::OK,
            IndexTemplate {
                list: Some(RunListView::new(
                    response,
                    limit,
                    offset,
                    initiated_by.clone(),
                    started_within.clone(),
                )),
                list_error: None,
                initiated_by,
                started_within,
            },
        ),
        Err(error) => {
            error!(%error, "failed to list runs from orchestrator");
            render_body(
                StatusCode::OK,
                IndexTemplate {
                    initiated_by,
                    started_within,
                    list: None,
                    list_error: Some(
                        "Could not load recent runs from the orchestrator.".to_owned(),
                    ),
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        endpoints::body_text,
        orchestrator::mocks::{MockClient, sample_summary},
    };
    use rtf_orchestrator_shared::status::Status;
    use uuid::Uuid;

    #[test]
    fn index_body_renders_the_recent_runs_table() {
        let run = sample_summary(Uuid::from_u128(1), Uuid::from_u128(2), Status::Running);
        let (status, body) = index_body(
            Ok(TestRunListResponse {
                runs: vec![run],
                total: 1,
            }),
            DEFAULT_LIMIT,
            0,
            String::new(),
            String::new(),
        );

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("my-test-run"),
            "recent-runs row should render"
        );
    }

    #[test]
    fn index_body_still_renders_the_lookup_form_when_listing_fails() {
        let (status, body) = index_body(
            Err(orchestrator::Error::ListRuns {
                status: StatusCode::BAD_GATEWAY,
            }),
            DEFAULT_LIMIT,
            0,
            String::new(),
            String::new(),
        );

        assert_eq!(status, StatusCode::OK, "the page itself still renders");
        assert!(
            body.contains("<form"),
            "run-id lookup box should survive a list failure"
        );
        assert!(body.contains("Could not load recent runs"));
    }

    #[tokio::test]
    async fn index_trims_whitespace_from_initiated_by() {
        let resp = handler(
            State(MockClient::with_test_run(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Status::Running,
            )),
            Query(IndexParams {
                initiated_by: Some("  testuser  ".to_owned()),
                started_within: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            body_text(resp).await.contains("value=\"testuser\""),
            "the trimmed value should repopulate the filter box"
        );
    }

    #[tokio::test]
    async fn index_falls_back_to_any_time_for_an_unrecognized_started_within() {
        let resp = handler(
            State(MockClient::with_test_run(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Status::Running,
            )),
            Query(IndexParams {
                initiated_by: None,
                started_within: Some("decade".to_owned()),
                offset: None,
            }),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "an unrecognized preset should not reject the request"
        );
    }
}
