use crate::{
    endpoints::{SelectedTemplate, to_response},
    links::LinksConfig,
    orchestrator::{self, Client},
    templates::{ErrorTemplate, RunNotFoundTemplate, RunTemplate},
    view::RunView,
};
use axum::{
    Extension,
    extract::{Path, Query, State},
    response::Response,
};
use chrono::{DateTime, Utc};
use rtf_orchestrator_shared::summary::TestRunSummary;
use tracing::error;
use uuid::Uuid;

#[derive(Debug, Default, serde::Deserialize)]
pub struct RunParams {
    /// A `Status` display value, e.g. `"FAILED"`.
    execution_status: Option<String>,
}

/// Also serves the htmx poll of `#run`. htmx doesn't swap non-2xx responses, so a failed poll
/// leaves the previous content in place.
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Extension(links_cfg): Extension<LinksConfig>,
    Path(id): Path<Uuid>,
    Query(params): Query<RunParams>,
) -> Response {
    let result = orchestrator_client.run_summary(id).await;
    let execution_status_filter = params.execution_status.unwrap_or_default();

    to_response(
        select_template(id, result, Utc::now(), &links_cfg, execution_status_filter).render(),
    )
}

fn select_template(
    id: Uuid,
    result: Result<Option<TestRunSummary>, orchestrator::Error>,
    now: DateTime<Utc>,
    links_cfg: &LinksConfig,
    execution_status_filter: String,
) -> SelectedTemplate<RunTemplate, RunNotFoundTemplate> {
    match result {
        Ok(Some(summary)) => SelectedTemplate::Found(Box::new(RunTemplate {
            run: RunView::new(summary, now, links_cfg, execution_status_filter),
        })),
        Ok(None) => SelectedTemplate::NotFound(RunNotFoundTemplate { id: id.to_string() }),
        Err(error) => {
            error!(%error, %id, "failed to fetch run summary from orchestrator");
            SelectedTemplate::Error(ErrorTemplate {
                message: format!("Could not load this run from the orchestrator: {error}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{links::sample_config, orchestrator::mocks::sample_summary};
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::status::Status;
    use std::assert_matches;

    #[test]
    fn selected_template_is_found_for_a_known_run() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let t = select_template(
            run_id,
            Ok(Some(sample_summary(run_id, ex_id, Status::Running))),
            Utc::now(),
            &sample_config(),
            "FAILED".to_owned(),
        );

        assert_matches!(t, SelectedTemplate::Found(_));
    }

    #[test]
    fn selected_template_is_not_found_for_an_unknown_run() {
        let id = Uuid::from_u128(1);
        let t = select_template(id, Ok(None), Utc::now(), &sample_config(), String::new());

        assert_matches!(t, SelectedTemplate::NotFound(_));
    }

    #[test]
    fn selected_template_is_an_error_when_the_fetch_fails() {
        let id = Uuid::from_u128(1);
        let t = select_template(
            id,
            Err(orchestrator::Error::TestRunStatus {
                status: StatusCode::BAD_GATEWAY,
                id,
            }),
            Utc::now(),
            &sample_config(),
            String::new(),
        );

        assert_matches!(t, SelectedTemplate::Error(_));
    }
}
