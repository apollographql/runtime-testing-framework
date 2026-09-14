use crate::{
    endpoints::{parse_trigger_ref_and_variables, render_body, to_response},
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
use reqwest::StatusCode;
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

/// Query params accepted by the known test plan detail page. `trigger_ref`/`trigger_variables`/
/// `trigger_error` are only ever set by [`redirect_with_trigger_error`] after a failed trigger
/// submission, to re-populate the form and show the failure inline. `trigger_ref` doubles as the
/// ref to preview details for: the trigger form's small "Preview" GET form re-submits the page at
/// `?trigger_ref=...` to re-resolve variables/matrix/services/environment for that ref, and the
/// POST trigger form carries the same value through as a hidden field so triggering always uses
/// whatever ref is currently being previewed. `days_back`/`days` page the history charts.
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

struct TestPlanDetailResults {
    plan: Result<Option<KnownTestPlanSummary>, orchestrator::Error>,
    runs: Result<TestRunListResponse, orchestrator::Error>,
    details: Result<Option<TestPlanDetails>, orchestrator::Error>,
}

/// `GET /ui/test-plan/{uuid}` — the plan's metadata (with a GitHub link), the executions
/// overview/services table/trigger form/history charts built from the orchestrator's details
/// endpoint, and a paginated table of its recent runs. The plan, its details, and its runs are all
/// fetched concurrently, since none of those three calls needs anything from either of the others.
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

    to_response(test_plan_detail_body(
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
    ))
}

/// Maps the plan lookup and runs-list results to a rendered `(status, body)` pair: the plan header,
/// trigger form, and runs table on success, a "not found" page for an unknown uuid, or an error
/// page for a plan-fetch failure. A runs-list failure still renders the page - with the plan header
/// and GitHub link intact and an inline error in place of the runs table - mirroring the home
/// page's `index_body`. `fetch.runs` is only rendered when `fetch.plan` is `Ok(Some(_))`; kept as
/// a plain field (rather than only fetched conditionally) so this whole mapping stays a single
/// pure, directly-testable function, matching `run_status_body`/`execution_detail_body`.
fn test_plan_detail_body(
    uuid: Uuid,
    fetch: TestPlanDetailResults,
    page: TestPlanDetailPage,
) -> (StatusCode, String) {
    let plan = match fetch.plan {
        Ok(Some(plan)) => plan,
        Ok(None) => {
            return render_body(
                StatusCode::NOT_FOUND,
                TestPlanNotFoundTemplate {
                    uuid: uuid.to_string(),
                },
            );
        }
        Err(error) => {
            error!(%error, %uuid, "failed to fetch known test plan from orchestrator");
            return render_body(
                StatusCode::BAD_GATEWAY,
                ErrorTemplate {
                    message: "Could not load this known test plan from the orchestrator."
                        .to_owned(),
                },
            );
        }
    };

    let plan_view = KnownTestPlanRowView::from(plan);

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
                Some("Could not load details for this test plan from the orchestrator.".to_owned()),
            )
        }
    };

    match fetch.runs {
        Ok(response) => render_body(
            StatusCode::OK,
            TestPlanDetailTemplate {
                plan: plan_view,
                details,
                details_error,
                runs: Some(RunListView::for_known_test_plan(
                    response,
                    page.limit,
                    page.offset,
                    uuid,
                )),
                runs_error: None,
                trigger_git_ref: page.trigger_git_ref,
                trigger_variables: page.trigger_variables,
                trigger_error: page.trigger_error,
                days_back: page.days_back,
                days: page.days,
            },
        ),
        Err(error) => {
            error!(%error, %uuid, "failed to list runs for known test plan from orchestrator");
            render_body(
                StatusCode::OK,
                TestPlanDetailTemplate {
                    plan: plan_view,
                    details,
                    details_error,
                    runs: None,
                    runs_error: Some("Could not load recent runs for this test plan.".to_owned()),
                    trigger_git_ref: page.trigger_git_ref,
                    trigger_variables: page.trigger_variables,
                    trigger_error: page.trigger_error,
                    days_back: page.days_back,
                    days: page.days,
                },
            )
        }
    }
}

/// Form fields submitted by the trigger form embedded on the known test plan detail page.
#[derive(Debug, Default, serde::Deserialize)]
pub struct KnownTestPlanTriggerForm {
    #[serde(rename = "ref")]
    git_ref: String,
    /// A JSON object of `HashMap<String, VariableOverride>` - a plain value templates a single
    /// variable, an array value defines a matrix dimension. Blank means no overrides.
    variables: String,
    #[serde(default)]
    days_back: Option<u32>,
    #[serde(default)]
    days: Option<u32>,
}

impl KnownTestPlanTriggerForm {
    /// Builds the orchestrator payload for `uuid`, or the message to show inline if `variables`
    /// isn't valid JSON. Trims both fields first, mirroring `trigger::TriggerForm::try_into_payload`.
    fn try_into_payload(&self, uuid: Uuid) -> Result<KnownTestPlanUuidPayload, String> {
        let (git_ref, variables) = parse_trigger_ref_and_variables(&self.git_ref, &self.variables)?;

        Ok(KnownTestPlanUuidPayload {
            test_plan_uuid: uuid,
            git_ref,
            variables,
        })
    }
}

/// `POST /ui/test-plan/{uuid}/trigger` — builds a [`KnownTestPlanUuidPayload`] from the submitted
/// form and POSTs it to the orchestrator's `test-run/trigger` endpoint, forwarding the caller's IAP
/// identity as with the GitHub trigger form. On success, redirects to the new run's status page. On
/// failure, redirects back to this plan's detail page with the submitted fields and the failure
/// message carried as query params - simpler than re-rendering the whole detail page directly here,
/// which would mean duplicating the `GET` handler's plan/runs fetching in the `POST` handler too.
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

/// Redirects back to the plan's detail page carrying the submitted form fields and the failure
/// message as query params, so the subsequent `GET` re-populates the form and shows the error
/// inline.
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
    use crate::{
        endpoints::body_text,
        orchestrator::mocks::{MockClient, sample_known_test_plan, sample_test_plan_details},
    };
    use axum::http::header::LOCATION;
    use rtf_orchestrator_shared::status::Status;

    #[test]
    fn test_plan_detail_body_renders_the_plan_and_its_runs() {
        let uuid = Uuid::from_u128(1);
        let (status, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Ok(Some(sample_known_test_plan(uuid))),
                runs: Ok(TestRunListResponse::default()),
                details: Ok(Some(sample_test_plan_details(uuid))),
            },
            TestPlanDetailPage::default(),
        );

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("my-known-test-plan"),
            "plan name should render"
        );
        assert!(
            body.contains("a sample known test plan"),
            "plan description should render"
        );
        assert!(
            body.contains(
                r#"href="https://github.com/apollographql/runtime-testing-framework/blob/abc1234def5678/test-plans/example.yaml""#
            ),
            "plan should link to its file on GitHub at the resolved sha, got: {body}"
        );
        assert!(
            body.contains("<form"),
            "expected the trigger form to render"
        );
        assert!(
            body.contains("Namespace write access") && body.contains("Disabled"),
            "expected namespace write access to render as disabled by default, got: {body}"
        );
    }

    #[test]
    fn test_plan_detail_body_shows_namespace_write_access_when_enabled() {
        let uuid = Uuid::from_u128(1);
        let (_, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Ok(Some(KnownTestPlanSummary {
                    allow_k8s_write: true,
                    ..sample_known_test_plan(uuid)
                })),
                runs: Ok(TestRunListResponse::default()),
                details: Ok(Some(sample_test_plan_details(uuid))),
            },
            TestPlanDetailPage::default(),
        );

        assert!(
            body.contains("Namespace write access") && body.contains("Enabled"),
            "expected namespace write access to render as enabled, got: {body}"
        );
    }

    #[test]
    fn test_plan_detail_body_renders_not_found_for_an_unknown_uuid() {
        let uuid = Uuid::from_u128(1);
        let (status, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Ok(None),
                runs: Ok(TestRunListResponse::default()),
                details: Ok(Some(sample_test_plan_details(uuid))),
            },
            TestPlanDetailPage::default(),
        );

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("not found"));
    }

    #[test]
    fn test_plan_detail_body_renders_error_when_the_plan_fetch_fails() {
        let uuid = Uuid::from_u128(1);
        let (status, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Err(orchestrator::Error::KnownTestPlanStatus {
                    status: StatusCode::BAD_GATEWAY,
                    uuid,
                }),
                runs: Ok(TestRunListResponse::default()),
                details: Ok(Some(sample_test_plan_details(uuid))),
            },
            TestPlanDetailPage::default(),
        );

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("went wrong"));
    }

    #[test]
    fn test_plan_detail_body_still_renders_the_plan_when_the_runs_fetch_fails() {
        let uuid = Uuid::from_u128(1);
        let (status, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Ok(Some(sample_known_test_plan(uuid))),
                runs: Err(orchestrator::Error::ListKnownTestPlanRuns {
                    status: StatusCode::BAD_GATEWAY,
                    uuid,
                }),
                details: Ok(Some(sample_test_plan_details(uuid))),
            },
            TestPlanDetailPage::default(),
        );

        assert_eq!(
            status,
            StatusCode::OK,
            "the plan header should still render"
        );
        assert!(body.contains("my-known-test-plan"));
        assert!(body.contains("Could not load recent runs"));
    }

    #[test]
    fn test_plan_detail_body_still_renders_the_plan_when_the_details_fetch_fails() {
        let uuid = Uuid::from_u128(1);
        let (status, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Ok(Some(sample_known_test_plan(uuid))),
                runs: Ok(TestRunListResponse::default()),
                details: Err(orchestrator::Error::TestPlanDetailsStatus {
                    status: StatusCode::BAD_GATEWAY,
                    uuid,
                }),
            },
            TestPlanDetailPage::default(),
        );

        assert_eq!(
            status,
            StatusCode::OK,
            "the plan header and runs table should still render"
        );
        assert!(body.contains("my-known-test-plan"));
        assert!(body.contains("Could not load details for this test plan"));
        assert!(
            body.contains(
                r#"href="https://github.com/apollographql/runtime-testing-framework/blob/HEAD/test-plans/example.yaml""#
            ),
            "should fall back to the plan's HEAD-based GitHub link when details failed to load, got: {body}"
        );
    }

    #[test]
    fn test_plan_detail_body_renders_a_dropdown_for_an_allowed_values_variable() {
        let uuid = Uuid::from_u128(1);
        let (status, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Ok(Some(sample_known_test_plan(uuid))),
                runs: Ok(TestRunListResponse::default()),
                details: Ok(Some(sample_test_plan_details(uuid))),
            },
            TestPlanDetailPage::default(),
        );

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("<select") && body.contains("multiple"),
            "expected a multi-select dropdown for the `tier` variable's allowed_values, got: {body}"
        );
        assert!(body.contains("enterprise"));
    }

    #[test]
    fn test_plan_detail_body_repopulates_the_trigger_form_and_shows_its_error() {
        let uuid = Uuid::from_u128(1);
        let (status, body) = test_plan_detail_body(
            uuid,
            TestPlanDetailResults {
                plan: Ok(Some(sample_known_test_plan(uuid))),
                runs: Ok(TestRunListResponse::default()),
                details: Ok(Some(sample_test_plan_details(uuid))),
            },
            TestPlanDetailPage {
                trigger_git_ref: "a-branch".to_owned(),
                trigger_variables: r#"{"key": "value"}"#.to_owned(),
                trigger_error: Some("unknown test plan".to_owned()),
                ..Default::default()
            },
        );

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("unknown test plan"),
            "expected the trigger error to render"
        );
        assert!(
            body.contains("a-branch"),
            "expected the submitted ref to repopulate the form"
        );
        assert!(
            // askama HTML-escapes the textarea's contents, so `"` becomes `&#34;`.
            body.contains("{&#34;key&#34;: &#34;value&#34;}"),
            "expected the submitted variables to repopulate the form, got: {body}"
        );
    }

    #[tokio::test]
    async fn handler_calls_the_client_and_renders_whatever_comes_back() {
        let uuid = Uuid::from_u128(3);
        let resp = handler(
            State(MockClient::with_test_run(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Status::Running,
            )),
            Path(uuid),
            Query(TestPlanDetailParams::default()),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(
            body.contains("my-known-test-plan"),
            "expected the mock client's sample known test plan to render"
        );
        assert!(
            body.contains("my-test-run"),
            "expected the mock client's sample run to render in the runs table"
        );
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
