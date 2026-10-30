use crate::{
    endpoints::{SelectedTemplate, to_response},
    links::LinksConfig,
    orchestrator::{self, Client, OrchestratorClient},
    templates::{ErrorTemplate, RunNotFoundTemplate, RunTemplate},
    view::RunView,
};
use axum::{
    Extension,
    extract::{Path, Query},
    response::{IntoResponse, Redirect, Response},
};
use chrono::{DateTime, Utc};
use rtf_orchestrator_shared::{summary::TestRunSummary, whoami::WhoAmIResponse};
use tracing::error;
use url::form_urlencoded;
use uuid::Uuid;

#[derive(Debug, Default, serde::Deserialize)]
pub struct RunParams {
    /// A `Status` display value, e.g. `"FAILED"`.
    execution_status: Option<String>,
    #[serde(default)]
    cancel_error: Option<String>,
}

/// Also serves the htmx poll of `#run`. htmx doesn't swap non-2xx responses, so a failed poll
/// leaves the previous content in place.
pub async fn handler<C: Client>(
    OrchestratorClient(client): OrchestratorClient<C>,
    Extension(links_cfg): Extension<LinksConfig>,
    Path(id): Path<Uuid>,
    Query(params): Query<RunParams>,
) -> Response {
    let result = client.run_summary(id).await;
    let can_cancel = match &result {
        Ok(Some(summary)) => cancel_permitted(&client, summary).await,
        _ => false,
    };
    let execution_status_filter = params.execution_status.unwrap_or_default();

    to_response(
        select_template(
            id,
            result,
            Utc::now(),
            &links_cfg,
            execution_status_filter,
            can_cancel,
            params.cancel_error,
        )
        .render(),
    )
}

/// Terminal runs can't be cancelled so we skip checking the user's permissions for them.
async fn cancel_permitted<C: Client>(client: &C, run: &TestRunSummary) -> bool {
    if run.current_status.is_terminal() {
        return false;
    }

    let plan = match run.test_plan_id {
        Some(uuid) => client.known_test_plan_summary(uuid).await.unwrap_or_else(|error| {
            error!(%error, id = %run.id, "failed to fetch the run's known test plan from orchestrator");
            None
        }),
        None => None,
    };
    let whoami = client.whoami().await.unwrap_or_else(|error| {
        error!(%error, "failed to identify the current user from orchestrator");
        WhoAmIResponse::Unknown
    });

    whoami.can_cancel(&run.initiated_by, plan.as_ref())
}

pub async fn post_cancel<C: Client>(
    OrchestratorClient(client): OrchestratorClient<C>,
    Path(id): Path<Uuid>,
) -> Response {
    match client.cancel_run(id).await {
        Ok(_) => Redirect::to(&format!("/ui/run/{id}")).into_response(),
        Err(error) => {
            error!(%error, %id, "failed to cancel run from the orchestrator");
            let message = match &error {
                orchestrator::Error::CancelRun { message, .. } => message.clone(),
                e => format!("Unable to cancel run: {e}"),
            };
            let mut qs = form_urlencoded::Serializer::new(String::new());
            qs.append_pair("cancel_error", &message);

            Redirect::to(&format!("/ui/run/{id}?{}", qs.finish())).into_response()
        }
    }
}

fn select_template(
    id: Uuid,
    result: Result<Option<TestRunSummary>, orchestrator::Error>,
    now: DateTime<Utc>,
    links_cfg: &LinksConfig,
    execution_status_filter: String,
    can_cancel: bool,
    cancel_error: Option<String>,
) -> SelectedTemplate<RunTemplate, RunNotFoundTemplate> {
    match result {
        Ok(Some(summary)) => SelectedTemplate::Found(Box::new(RunTemplate {
            run: RunView {
                can_cancel,
                cancel_error,
                ..RunView::new(summary, now, links_cfg, execution_status_filter)
            },
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
    use crate::{
        links::sample_config,
        orchestrator::mocks::{MockClient, sample_summary},
    };
    use axum::http::header::LOCATION;
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::status::Status;
    use simple_test_case::test_case;
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
            false,
            None,
        );

        assert_matches!(t, SelectedTemplate::Found(_));
    }

    #[test]
    fn selected_template_carries_the_cancel_state_through_to_the_view() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let run = select_template(
            run_id,
            Ok(Some(sample_summary(run_id, ex_id, Status::Running))),
            Utc::now(),
            &sample_config(),
            String::new(),
            true,
            Some("run is no longer in flight".to_owned()),
        )
        .unwrap_found()
        .run;

        assert!(run.can_cancel);
        assert_eq!(
            run.cancel_error.as_deref(),
            Some("run is no longer in flight")
        );
    }

    #[test]
    fn selected_template_is_not_found_for_an_unknown_run() {
        let id = Uuid::from_u128(1);
        let t = select_template(
            id,
            Ok(None),
            Utc::now(),
            &sample_config(),
            String::new(),
            false,
            None,
        );

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
                message: "bad gateway".to_owned(),
            }),
            Utc::now(),
            &sample_config(),
            String::new(),
            false,
            None,
        );

        assert_matches!(t, SelectedTemplate::Error(_));
    }

    fn admin() -> WhoAmIResponse {
        WhoAmIResponse::Admin {
            email: "admin@test.com".to_owned(),
        }
    }

    #[test_case(Status::Running, admin(), true; "admin of a running run")]
    #[test_case(Status::Successful, admin(), false; "admin of a terminal run")]
    #[test_case(Status::Running, WhoAmIResponse::Unknown, false; "unknown user")]
    #[tokio::test]
    async fn cancel_permitted_cases(status: Status, whoami: WhoAmIResponse, expected: bool) {
        let run = sample_summary(Uuid::from_u128(1), Uuid::from_u128(2), status);
        let client =
            MockClient::with_test_run(run.id, Uuid::from_u128(2), status).with_whoami(whoami);

        assert_eq!(cancel_permitted(&client, &run).await, expected);
    }

    #[tokio::test]
    async fn post_cancel_redirects_to_the_run_on_success() {
        let run_id = Uuid::from_u128(1);
        let client = MockClient::with_test_run(run_id, Uuid::from_u128(2), Status::Running);

        let resp = post_cancel(OrchestratorClient(client), Path(run_id)).await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            resp.headers().get(LOCATION).unwrap(),
            &format!("/ui/run/{run_id}")
        );
    }

    #[tokio::test]
    async fn post_cancel_redirects_with_the_error_message_on_failure() {
        let run_id = Uuid::from_u128(1);
        let client = MockClient::with_test_run(run_id, Uuid::from_u128(2), Status::Running)
            .with_failing_cancel();

        let resp = post_cancel(OrchestratorClient(client), Path(run_id)).await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            resp.headers().get(LOCATION).unwrap(),
            &format!("/ui/run/{run_id}?cancel_error=run+is+no+longer+in+flight")
        );
    }
}
