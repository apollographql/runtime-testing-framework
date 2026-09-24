use crate::{
    endpoints::{SelectedTemplate, to_response},
    links::LinksConfig,
    orchestrator::{self, Client},
    templates::{ErrorTemplate, ExecutionNotFoundTemplate, ExecutionTemplate},
    view::ExecutionDetailView,
};
use axum::{
    Extension,
    extract::{Path, State},
    response::Response,
};
use rtf_orchestrator_shared::summary::TestExecutionSummary;
use tracing::error;
use uuid::Uuid;

pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Extension(links_cfg): Extension<LinksConfig>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    let result = orchestrator_client.execution_summary(execution_id).await;

    to_response(select_template(execution_id, result, &links_cfg).render())
}

fn select_template(
    execution_id: Uuid,
    result: Result<Option<TestExecutionSummary>, orchestrator::Error>,
    links_cfg: &LinksConfig,
) -> SelectedTemplate<ExecutionTemplate, ExecutionNotFoundTemplate> {
    match result {
        Ok(Some(execution)) => SelectedTemplate::Found(Box::new(ExecutionTemplate {
            execution: ExecutionDetailView::new(execution, links_cfg),
        })),
        Ok(None) => SelectedTemplate::NotFound(ExecutionNotFoundTemplate {
            execution_id: execution_id.to_string(),
        }),
        Err(error) => {
            error!(%error, %execution_id, "failed to fetch execution summary from orchestrator");
            SelectedTemplate::Error(ErrorTemplate {
                message: format!("Could not load this execution from the orchestrator: {error}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{links::sample_config, orchestrator::mocks::sample_execution};
    use reqwest::StatusCode;
    use std::assert_matches;

    #[test]
    fn selected_template_is_found_for_a_known_execution() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let t = select_template(
            ex_id,
            Ok(Some(sample_execution(run_id, ex_id))),
            &sample_config(),
        );

        assert_matches!(t, SelectedTemplate::Found(_));
    }

    #[test]
    fn selected_template_is_not_found_for_an_unknown_execution() {
        let ex_id = Uuid::from_u128(2);
        let t = select_template(ex_id, Ok(None), &sample_config());

        assert_matches!(t, SelectedTemplate::NotFound(_));
    }

    #[test]
    fn selected_template_is_an_error_when_the_fetch_fails() {
        let ex_id = Uuid::from_u128(2);
        let t = select_template(
            ex_id,
            Err(orchestrator::Error::TestExecutionStatus {
                status: StatusCode::BAD_GATEWAY,
                id: ex_id,
            }),
            &sample_config(),
        );

        assert_matches!(t, SelectedTemplate::Error(_));
    }
}
