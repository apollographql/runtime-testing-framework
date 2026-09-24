use crate::{
    endpoints::{SelectedTemplate, parse_trigger_ref_and_variables, to_response},
    orchestrator::{self, Client, IAP_USER_EMAIL_HEADER},
    templates::{ErrorTemplate, TestPlanDetailTemplate, TestPlanNotFoundTemplate},
    view::{KnownTestPlanRowView, RunListView, TestPlanDetailsView},
};
use axum::{
    Form,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use rtf_orchestrator_shared::{
    known_test_plan::{KnownTestPlanRunsParams, KnownTestPlanSummary},
    payload::{KnownTestPlanUuidPayload, TriggerPayload},
    summary::TestRunListResponse,
    test_plan_details::{DEFAULT_DAYS, DEFAULT_DAYS_BACK, TestPlanDetails, TestPlanDetailsParams},
};
use tracing::error;
use url::form_urlencoded;
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 20;

/// `trigger_variables`/`trigger_error` are only set by [`redirect_with_trigger_error`]. `trigger_ref`
/// is also the ref that details are resolved for, and must be carried through to the trigger form.
#[derive(Debug, Default, serde::Deserialize)]
pub struct TestPlanDetailParams {
    offset: Option<i64>,
    #[serde(default, rename = "trigger_ref")]
    trigger_git_ref: Option<String>,
    #[serde(default)]
    trigger_variables: Option<String>,
    #[serde(default)]
    trigger_error: Option<String>,
    #[serde(default)]
    days_back: Option<u32>,
    #[serde(default)]
    days: Option<u32>,
}

#[derive(Debug)]
struct TestPlanDetailPage {
    limit: i64,
    offset: i64,
    trigger_git_ref: String,
    trigger_variables: String,
    trigger_error: Option<String>,
    days_back: u32,
    days: u32,
}

impl Default for TestPlanDetailPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            offset: 0,
            trigger_git_ref: String::new(),
            trigger_variables: String::new(),
            trigger_error: None,
            days_back: DEFAULT_DAYS_BACK,
            days: DEFAULT_DAYS,
        }
    }
}

#[derive(Debug)]
struct TestPlanDetailResults {
    plan: Result<Option<KnownTestPlanSummary>, orchestrator::Error>,
    runs: Result<TestRunListResponse, orchestrator::Error>,
    details: Result<Option<TestPlanDetails>, orchestrator::Error>,
}

pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Path(uuid): Path<Uuid>,
    Query(params): Query<TestPlanDetailParams>,
) -> Response {
    let offset = params.offset.unwrap_or(0).max(0);
    let runs_params = KnownTestPlanRunsParams {
        limit: Some(DEFAULT_LIMIT),
        offset: Some(offset),
        ..Default::default()
    };
    let days_back = params.days_back.unwrap_or(DEFAULT_DAYS_BACK);
    let days = params.days.unwrap_or(DEFAULT_DAYS);
    let details_params = TestPlanDetailsParams {
        days_back,
        days,
        git_ref: params.trigger_git_ref.clone(),
    };

    let (plan, runs, details) = tokio::join!(
        orchestrator_client.known_test_plan_summary(uuid),
        orchestrator_client.list_known_test_plan_runs(uuid, &runs_params),
        orchestrator_client.test_plan_details(uuid, &details_params)
    );

    to_response(
        select_template(
            uuid,
            TestPlanDetailResults {
                plan,
                runs,
                details,
            },
            TestPlanDetailPage {
                limit: DEFAULT_LIMIT,
                offset,
                trigger_git_ref: params.trigger_git_ref.unwrap_or_default(),
                trigger_variables: params.trigger_variables.unwrap_or_default(),
                trigger_error: params.trigger_error,
                days_back,
                days,
            },
        )
        .render(),
    )
}

fn select_template(
    uuid: Uuid,
    fetch: TestPlanDetailResults,
    page: TestPlanDetailPage,
) -> SelectedTemplate<TestPlanDetailTemplate, TestPlanNotFoundTemplate> {
    let plan = match fetch.plan {
        Ok(Some(plan)) => plan,
        Ok(None) => {
            return SelectedTemplate::NotFound(TestPlanNotFoundTemplate {
                uuid: uuid.to_string(),
            });
        }
        Err(error) => {
            error!(%error, %uuid, "failed to fetch known test plan from orchestrator");
            return SelectedTemplate::Error(ErrorTemplate {
                message: format!(
                    "Could not load this known test plan from the orchestrator: {error}"
                ),
            });
        }
    };

    let (details, details_error) = match fetch.details {
        Ok(Some(details)) => (
            Some(TestPlanDetailsView::new(details, page.days_back, page.days)),
            None,
        ),
        Ok(None) => {
            error!(%uuid, "test plan details endpoint returned 404 for an already-known test plan");
            (
                None,
                Some("Could not load details for this test plan from the orchestrator.".to_owned()),
            )
        }
        Err(error) => {
            error!(%error, %uuid, "failed to fetch test plan details from orchestrator");
            (
                None,
                Some(format!(
                    "Could not load details for this test plan from the orchestrator: {error}"
                )),
            )
        }
    };

    let (runs, runs_error) = match fetch.runs {
        Ok(response) => (
            Some(RunListView::for_known_test_plan(
                response,
                page.limit,
                page.offset,
                uuid,
            )),
            None,
        ),
        Err(error) => {
            error!(%error, %uuid, "failed to list runs for known test plan from orchestrator");
            (
                None,
                Some(format!(
                    "Could not load recent runs for this test plan: {error}"
                )),
            )
        }
    };

    SelectedTemplate::Found(Box::new(TestPlanDetailTemplate {
        plan: KnownTestPlanRowView::from(plan),
        details,
        details_error,
        runs,
        runs_error,
        trigger_git_ref: page.trigger_git_ref,
        trigger_variables: page.trigger_variables,
        trigger_error: page.trigger_error,
        days_back: page.days_back,
        days: page.days,
    }))
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct KnownTestPlanTriggerForm {
    #[serde(rename = "ref")]
    git_ref: String,
    /// JSON object of variable overrides, where an array value defines a matrix dimension.
    variables: String,
    #[serde(default)]
    days_back: Option<u32>,
    #[serde(default)]
    days: Option<u32>,
}

impl KnownTestPlanTriggerForm {
    fn try_into_payload(&self, uuid: Uuid) -> Result<KnownTestPlanUuidPayload, String> {
        let (git_ref, variables) = parse_trigger_ref_and_variables(&self.git_ref, &self.variables)?;

        Ok(KnownTestPlanUuidPayload {
            test_plan_uuid: uuid,
            git_ref,
            variables,
        })
    }
}

pub async fn post_trigger<C: Client>(
    State(orchestrator_client): State<C>,
    headers: HeaderMap,
    Path(uuid): Path<Uuid>,
    Form(form): Form<KnownTestPlanTriggerForm>,
) -> Response {
    let payload = match form.try_into_payload(uuid) {
        Ok(payload) => payload,
        Err(message) => return redirect_with_trigger_error(uuid, &form, &message),
    };

    let initiated_by = headers
        .get(IAP_USER_EMAIL_HEADER)
        .and_then(|value| value.to_str().ok());

    match orchestrator_client
        .trigger(&TriggerPayload::KnownTestPlanUuid(payload), initiated_by)
        .await
    {
        Ok(summary) => Redirect::to(&format!("/ui/run/{}", summary.id)).into_response(),
        Err(error) => {
            error!(%error, %uuid, "failed to trigger a run of a known test plan from the orchestrator");
            let message = match &error {
                orchestrator::Error::Trigger { message, .. } => message.clone(),
                e => format!("Unable to trigger run: {e}"),
            };
            redirect_with_trigger_error(uuid, &form, &message)
        }
    }
}

fn redirect_with_trigger_error(
    uuid: Uuid,
    form: &KnownTestPlanTriggerForm,
    message: &str,
) -> Response {
    let mut qs = form_urlencoded::Serializer::new(String::new());
    if !form.git_ref.is_empty() {
        qs.append_pair("trigger_ref", &form.git_ref);
    }
    if !form.variables.is_empty() {
        qs.append_pair("trigger_variables", &form.variables);
    }
    if let Some(days_back) = form.days_back {
        qs.append_pair("days_back", &days_back.to_string());
    }
    if let Some(days) = form.days {
        qs.append_pair("days", &days.to_string());
    }
    qs.append_pair("trigger_error", message);

    Redirect::to(&format!("/ui/test-plan/{uuid}?{}", qs.finish())).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::mocks::{
        MockClient, sample_known_test_plan, sample_test_plan_details,
    };
    use axum::http::header::LOCATION;
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::status::Status;
    use std::assert_matches;

    fn ok_results(uuid: Uuid) -> TestPlanDetailResults {
        TestPlanDetailResults {
            plan: Ok(Some(sample_known_test_plan(uuid))),
            runs: Ok(TestRunListResponse::default()),
            details: Ok(Some(sample_test_plan_details(uuid))),
        }
    }

    #[test]
    fn selected_template_is_found_for_a_known_test_plan() {
        let uuid = Uuid::from_u128(1);
        let t = select_template(uuid, ok_results(uuid), TestPlanDetailPage::default());

        assert_matches!(t, SelectedTemplate::Found(_));
    }

    #[test]
    fn selected_template_is_not_found_for_an_unknown_test_plan() {
        let uuid = Uuid::from_u128(1);
        let t = select_template(
            uuid,
            TestPlanDetailResults {
                plan: Ok(None),
                ..ok_results(uuid)
            },
            TestPlanDetailPage::default(),
        );

        assert_matches!(t, SelectedTemplate::NotFound(_));
    }

    #[test]
    fn selected_template_is_an_error_when_the_plan_fetch_fails() {
        let uuid = Uuid::from_u128(1);
        let t = select_template(
            uuid,
            TestPlanDetailResults {
                plan: Err(orchestrator::Error::KnownTestPlanStatus {
                    status: StatusCode::BAD_GATEWAY,
                    uuid,
                }),
                ..ok_results(uuid)
            },
            TestPlanDetailPage::default(),
        );

        assert_matches!(t, SelectedTemplate::Error(_));
    }

    #[test]
    fn selected_template_is_an_found_with_runs_error_when_the_runs_fetch_fails() {
        let uuid = Uuid::from_u128(1);
        let t = select_template(
            uuid,
            TestPlanDetailResults {
                runs: Err(orchestrator::Error::ListKnownTestPlanRuns {
                    status: StatusCode::BAD_GATEWAY,
                    uuid,
                }),
                ..ok_results(uuid)
            },
            TestPlanDetailPage::default(),
        )
        .unwrap_found();

        assert!(t.runs_error.is_some());
    }

    #[test]
    fn selected_template_is_found_with_details_error_when_the_details_fetch_fails() {
        let uuid = Uuid::from_u128(1);
        let t = select_template(
            uuid,
            TestPlanDetailResults {
                details: Err(orchestrator::Error::TestPlanDetailsStatus {
                    status: StatusCode::BAD_GATEWAY,
                    uuid,
                }),
                ..ok_results(uuid)
            },
            TestPlanDetailPage::default(),
        )
        .unwrap_found();

        assert!(t.details_error.is_some());
    }

    #[test]
    fn test_plan_detail_body_repopulates_the_trigger_form_and_shows_its_error() {
        let uuid = Uuid::from_u128(1);
        let t = select_template(
            uuid,
            ok_results(uuid),
            TestPlanDetailPage {
                trigger_error: Some("unknown test plan".to_owned()),
                ..Default::default()
            },
        )
        .unwrap_found();

        assert_eq!(t.trigger_error.as_deref(), Some("unknown test plan"));
    }

    fn sample_form() -> KnownTestPlanTriggerForm {
        KnownTestPlanTriggerForm {
            git_ref: String::new(),
            variables: String::new(),
            days_back: None,
            days: None,
        }
    }

    #[test]
    fn try_into_payload_wires_the_uuid_and_parsed_ref_and_variables_into_the_payload() {
        let uuid = Uuid::from_u128(1);
        let form = KnownTestPlanTriggerForm {
            git_ref: "  main  ".to_owned(),
            variables: r#"{"message": "hello"}"#.to_owned(),
            ..sample_form()
        };
        let payload = form.try_into_payload(uuid).expect("should parse");

        assert_eq!(payload.test_plan_uuid, uuid);
        assert_eq!(payload.git_ref, Some("main".to_owned()));
        assert!(payload.variables.is_some());
    }

    #[test]
    fn redirect_with_trigger_error_carries_the_message_and_fields_as_query_params() {
        let uuid = Uuid::from_u128(1);
        let form = KnownTestPlanTriggerForm {
            git_ref: "a branch".to_owned(),
            variables: r#"{"a":1}"#.to_owned(),
            ..sample_form()
        };
        let resp = redirect_with_trigger_error(uuid, &form, "unknown test plan");

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let location = resp
            .headers()
            .get(LOCATION)
            .expect("redirect should carry a Location header")
            .to_str()
            .unwrap();

        assert!(location.starts_with(&format!("/ui/test-plan/{uuid}?")));
        assert!(location.contains("trigger_ref=a+branch"));
        assert!(location.contains("trigger_error=unknown+test+plan"));
    }

    #[test]
    fn redirect_with_trigger_error_carries_the_history_window_when_present() {
        let uuid = Uuid::from_u128(1);
        let form = KnownTestPlanTriggerForm {
            days_back: Some(60),
            days: Some(30),
            ..sample_form()
        };
        let resp = redirect_with_trigger_error(uuid, &form, "unknown test plan");

        let location = resp
            .headers()
            .get(LOCATION)
            .expect("redirect should carry a Location header")
            .to_str()
            .unwrap()
            .to_owned();

        assert!(
            location.contains("days_back=60"),
            "expected the history window to survive the redirect, got: {location}"
        );
        assert!(location.contains("days=30"));
    }

    #[tokio::test]
    async fn post_trigger_returns_a_redirect_without_calling_the_client_when_variables_are_invalid()
    {
        let uuid = Uuid::from_u128(1);
        let resp = post_trigger(
            State(MockClient::with_test_run(
                Uuid::from_u128(2),
                Uuid::from_u128(3),
                Status::Running,
            )),
            HeaderMap::new(),
            Path(uuid),
            Form(KnownTestPlanTriggerForm {
                variables: "not json".to_owned(),
                ..sample_form()
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let location = resp
            .headers()
            .get(LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(location.starts_with(&format!("/ui/test-plan/{uuid}?")));
        assert!(location.contains("trigger_error="));
    }

    #[tokio::test]
    async fn post_trigger_calls_the_client_and_redirects_to_the_new_run_on_success() {
        let run_id = Uuid::from_u128(1);
        let ex_id = Uuid::from_u128(2);
        let resp = post_trigger(
            State(MockClient::with_test_run(run_id, ex_id, Status::Running)),
            HeaderMap::new(),
            Path(Uuid::from_u128(3)),
            Form(sample_form()),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            resp.headers().get(LOCATION).unwrap(),
            &format!("/ui/run/{run_id}")
        );
    }
}
