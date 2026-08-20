//! `GET`/`POST /ui/trigger` — trigger a run from a GitHub-hosted test plan.
use crate::{
    endpoints::{parse_trigger_ref_and_variables, render_body, to_response},
    orchestrator::{self, Client, IAP_USER_EMAIL_HEADER},
    templates::TriggerTemplate,
};
use axum::{
    Form,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use reqwest::StatusCode;
use rtf_orchestrator_shared::{
    payload::{GitHubPayload, TriggerPayload},
    summary::TestRunSummary,
};
use serde::Deserialize;
use tracing::error;

/// `GET /ui/trigger` - the "trigger a run from a GitHub-hosted test plan" form.
pub async fn get_handler() -> Response {
    to_response(render_body(StatusCode::OK, TriggerTemplate::default()))
}

/// `POST /ui/trigger` - builds a [`GitHubPayload`] from the submitted form and POSTs it to the
/// orchestrator's `test-run/trigger` endpoint, forwarding the caller's IAP identity so the run's
/// `initiated_by` reflects who submitted the form rather than `"unknown"`.
pub async fn post_handler<C: Client>(
    State(orchestrator_client): State<C>,
    headers: HeaderMap,
    Form(form): Form<TriggerForm>,
) -> Response {
    let payload = match form.try_into_payload() {
        Ok(payload) => payload,
        Err(message) => {
            return to_response(render_body(
                StatusCode::BAD_REQUEST,
                TriggerTemplate {
                    error: Some(message),
                    ..form.into()
                },
            ));
        }
    };

    let initiated_by = headers
        .get(IAP_USER_EMAIL_HEADER)
        .and_then(|value| value.to_str().ok());

    let result = orchestrator_client
        .trigger(&TriggerPayload::GitHub(payload), initiated_by)
        .await;

    trigger_result_response(form, result)
}

/// Form fields submitted by the "trigger a run from GitHub" form.
#[derive(Debug, Default, Deserialize)]
pub struct TriggerForm {
    org: String,
    repo: String,
    path: String,
    #[serde(rename = "ref")]
    git_ref: String,
    /// A JSON object of `HashMap<String, ScalarOrArray>` - a plain value templates a single
    /// variable, an array value defines a matrix dimension. Blank means no overrides.
    variables: String,
}

impl TriggerForm {
    /// Builds the orchestrator payload, or the message to show inline if `variables` isn't valid
    /// JSON. Trims every field first, so stray leading/trailing whitespace doesn't turn into a
    /// bogus org/repo/path or a spuriously "non-blank" variables box.
    fn try_into_payload(&self) -> Result<GitHubPayload, String> {
        let (git_ref, variables) = parse_trigger_ref_and_variables(&self.git_ref, &self.variables)?;

        Ok(GitHubPayload {
            org: self.org.trim().to_owned(),
            repo: self.repo.trim().to_owned(),
            path: self.path.trim().to_owned(),
            git_ref,
            variables,
        })
    }
}

impl From<TriggerForm> for TriggerTemplate {
    fn from(form: TriggerForm) -> Self {
        Self {
            org: form.org,
            repo: form.repo,
            path: form.path,
            git_ref: form.git_ref,
            variables: form.variables,
            error: None,
        }
    }
}

/// Maps the result of triggering a run to a response: a redirect to the new run's status page on
/// success, or the form re-rendered with its fields intact and an inline error otherwise. Kept
/// free of the orchestrator [`Client`] so it's testable directly against hand-built results.
fn trigger_result_response(
    form: TriggerForm,
    result: Result<TestRunSummary, orchestrator::Error>,
) -> Response {
    match result {
        Ok(summary) => Redirect::to(&format!("/ui/run/{}", summary.id)).into_response(),

        Err(error) => {
            error!(%error, "failed to trigger a run from the orchestrator");
            let (status, message) = match error {
                orchestrator::Error::Trigger { status, message } => (status, message),
                e => (
                    StatusCode::BAD_GATEWAY,
                    format!("Unable to trigger run: {e}"),
                ),
            };

            to_response(render_body(
                status,
                TriggerTemplate {
                    error: Some(message),
                    ..form.into()
                },
            ))
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
    use axum::http::header::LOCATION;
    use rtf_orchestrator_shared::status::Status;
    use uuid::Uuid;

    fn sample_form() -> TriggerForm {
        TriggerForm {
            org: "apollographql".to_owned(),
            repo: "runtime-testing-framework".to_owned(),
            path: "test-plans/smoke/test-plan.yaml".to_owned(),
            git_ref: String::new(),
            variables: String::new(),
        }
    }

    #[test]
    fn try_into_payload_trims_fields_and_wires_the_parsed_ref_and_variables_into_the_payload() {
        let form = TriggerForm {
            org: "  apollographql  ".to_owned(),
            repo: "  runtime-testing-framework  ".to_owned(),
            path: "  test-plans/smoke/test-plan.yaml  ".to_owned(),
            git_ref: "  main  ".to_owned(),
            variables: r#"{"message": "hello"}"#.to_owned(),
        };
        let payload = form.try_into_payload().expect("should parse");

        assert_eq!(payload.org, "apollographql");
        assert_eq!(payload.repo, "runtime-testing-framework");
        assert_eq!(payload.path, "test-plans/smoke/test-plan.yaml");
        assert_eq!(payload.git_ref, Some("main".to_owned()));
        assert!(payload.variables.is_some());
    }

    #[tokio::test]
    async fn get_handler_renders_the_empty_form() {
        let resp = get_handler().await;

        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_text(resp).await;

        assert!(
            body.contains("<form"),
            "expected a form on the trigger page"
        );
        assert!(body.contains(r#"name="org""#));
        assert!(body.contains(r#"name="variables""#));
    }

    #[test]
    fn trigger_result_response_redirects_to_the_new_run_on_success() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = trigger_result_response(
            sample_form(),
            Ok(sample_summary(run_id, ex_id, Status::Initialising)),
        );

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            resp.headers().get(LOCATION).unwrap(),
            &format!("/ui/run/{run_id}")
        );
    }

    #[tokio::test]
    async fn trigger_result_response_re_renders_the_form_with_the_orchestrators_error() {
        let resp = trigger_result_response(
            sample_form(),
            Err(orchestrator::Error::Trigger {
                status: StatusCode::BAD_REQUEST,
                message: "unknown path in repo".to_owned(),
            }),
        );

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = body_text(resp).await;

        assert!(body.contains("unknown path in repo"));
        assert!(
            body.contains("apollographql"),
            "form fields should be repopulated, got: {body}"
        );
    }

    #[tokio::test]
    async fn trigger_result_response_falls_back_to_bad_gateway_on_an_unexpected_error() {
        // `ListRuns` never actually comes back from `trigger` - it stands in here for any
        // non-`Trigger` variant, to exercise the fallback arm of the match.
        let resp = trigger_result_response(
            sample_form(),
            Err(orchestrator::Error::ListRuns {
                status: StatusCode::INTERNAL_SERVER_ERROR,
            }),
        );

        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        assert!(body_text(resp).await.contains("Unable to trigger run"));
    }

    #[tokio::test]
    async fn post_handler_returns_bad_request_without_calling_the_client_when_variables_are_invalid()
     {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = post_handler(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            HeaderMap::new(),
            Form(TriggerForm {
                variables: "not json".to_owned(),
                ..sample_form()
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_handler_calls_the_client_and_redirects_on_success() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = post_handler(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            HeaderMap::new(),
            Form(sample_form()),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    }
}
