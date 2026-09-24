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

#[derive(Debug, serde::Deserialize)]
pub struct IndexParams {
    initiated_by: Option<String>,
    started_within: Option<String>,
    offset: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartedWithin {
    Hour,
    Day,
    Week,
    Month,
}

impl StartedWithin {
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

pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Query(params): Query<IndexParams>,
) -> Response {
    let offset = params.offset.unwrap_or(0).max(0);
    // Untrimmed, a stray space would filter on a value that matches nothing.
    let initiated_by = params
        .initiated_by
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let started_within = params.started_within.unwrap_or_default();

    let filter = RunListFilter {
        initiated_by: initiated_by.clone(),
        started_after: StartedWithin::parse(&started_within).map(|w| Utc::now() - w.as_duration()),
        limit: DEFAULT_LIMIT,
        offset,
    };

    let list = orchestrator_client.list_runs(&filter).await;

    to_response(render_body(
        StatusCode::OK,
        index_template(list, DEFAULT_LIMIT, offset, initiated_by, started_within),
    ))
}

fn index_template(
    result: Result<TestRunListResponse, orchestrator::Error>,
    limit: i64,
    offset: i64,
    initiated_by: String,
    started_within: String,
) -> IndexTemplate {
    match result {
        Ok(response) => IndexTemplate {
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
        Err(error) => {
            error!(%error, "failed to list runs from orchestrator");
            IndexTemplate {
                initiated_by,
                started_within,
                list: None,
                list_error: Some("Could not load recent runs from the orchestrator.".to_owned()),
            }
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
    fn index_template_carries_the_recent_runs_and_filters() {
        let run = sample_summary(Uuid::from_u128(1), Uuid::from_u128(2), Status::Running);
        let t = index_template(
            Ok(TestRunListResponse {
                runs: vec![run],
                total: 1,
            }),
            DEFAULT_LIMIT,
            0,
            "testuser".to_owned(),
            "week".to_owned(),
        );

        let list = t.list.expect("the runs list should be present");
        assert_eq!(list.rows.len(), 1);
        assert_eq!(t.list_error, None);
        assert_eq!(t.initiated_by, "testuser");
        assert_eq!(t.started_within, "week");
    }

    #[test]
    fn index_template_carries_an_inline_error_when_listing_fails() {
        let t = index_template(
            Err(orchestrator::Error::ListRuns {
                status: StatusCode::BAD_GATEWAY,
            }),
            DEFAULT_LIMIT,
            0,
            String::new(),
            String::new(),
        );

        assert!(t.list.is_none());
        assert!(t.list_error.is_some());
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
