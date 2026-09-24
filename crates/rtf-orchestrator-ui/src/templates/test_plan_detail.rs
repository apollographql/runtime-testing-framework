use crate::view::{EnvironmentDetailView, KnownTestPlanRowView, RunListView, TestPlanDetailsView};
use askama::Template;

#[derive(Debug, Template)]
#[template(
    path = "test_plan_detail.html",
    blocks = ["plan_header", "overview", "trigger_form", "runs"]
)]
pub struct TestPlanDetailTemplate {
    pub plan: KnownTestPlanRowView,
    pub details: Option<TestPlanDetailsView>,
    pub details_error: Option<String>,
    pub runs: Option<RunListView>,
    pub runs_error: Option<String>,
    pub trigger_git_ref: String,
    pub days_back: u32,
    pub days: u32,
    pub trigger_variables: String,
    pub trigger_error: Option<String>,
}

impl TestPlanDetailTemplate {
    /// Falls back to `HEAD` when details failed to load.
    pub fn github_url(&self) -> &str {
        self.details
            .as_ref()
            .map_or(&self.plan.github_url, |d| &d.github_url)
    }

    pub fn write_access_label(&self) -> &'static str {
        if self.plan.allow_k8s_write {
            "Enabled"
        } else {
            "Disabled"
        }
    }

    pub fn customize_expanded(&self) -> bool {
        self.trigger_error.is_some() || !self.trigger_variables.is_empty()
    }

    pub fn focus_variables(&self) -> bool {
        self.trigger_error.is_some()
    }
}

#[derive(Debug, Template)]
#[template(path = "test_plan_not_found.html")]
pub struct TestPlanNotFoundTemplate {
    pub uuid: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{orchestrator::mocks, preview, render_fragment};
    use chrono::{TimeZone, Utc};
    use rtf_orchestrator_shared::{status::Status, summary::TestRunListResponse};
    use uuid::Uuid;

    fn details_failed() -> TestPlanDetailTemplate {
        TestPlanDetailTemplate {
            details: None,
            details_error: Some("Could not load details for this test plan.".to_owned()),
            ..preview::test_plan_detail()
        }
    }

    #[test]
    fn github_url_uses_the_resolved_sha_when_details_loaded() {
        assert_eq!(
            preview::test_plan_detail().github_url(),
            "https://github.com/apollographql/runtime-testing-framework/blob/abc1234def5678/test-plans/example.yaml",
        );
    }

    #[test]
    fn github_url_falls_back_to_head_without_details() {
        assert_eq!(
            details_failed().github_url(),
            "https://github.com/apollographql/runtime-testing-framework/blob/HEAD/test-plans/example.yaml",
        );
    }

    #[test]
    fn write_access_label_is_disabled_by_default() {
        assert_eq!(preview::test_plan_detail().write_access_label(), "Disabled");
    }

    #[test]
    fn write_access_label_is_enabled_when_k8s_write_is_allowed() {
        let mut t = preview::test_plan_detail();
        t.plan.allow_k8s_write = true;

        assert_eq!(t.write_access_label(), "Enabled");
    }

    #[test]
    fn customize_expanded_and_focus_variables_on_error_with_no_variables_submitted() {
        let t = TestPlanDetailTemplate {
            trigger_variables: String::new(),
            trigger_error: Some("unknown ref".to_owned()),
            ..preview::test_plan_detail()
        };

        assert!(t.customize_expanded());
        assert!(t.focus_variables());
    }

    #[test]
    fn plan_header_snapshot() {
        insta::assert_snapshot!(render_fragment!(
            preview::test_plan_detail().as_plan_header()
        ));
    }

    #[test]
    fn plan_header_snapshot_without_details() {
        insta::assert_snapshot!(render_fragment!(details_failed().as_plan_header()));
    }

    #[test]
    fn overview_snapshot_docker_compose() {
        insta::assert_snapshot!(render_fragment!(preview::test_plan_detail().as_overview()));
    }

    #[test]
    fn overview_snapshot_k8s() {
        insta::assert_snapshot!(render_fragment!(
            preview::test_plan_detail_k8s().as_overview()
        ));
    }

    #[test]
    fn overview_snapshot_details_failed() {
        insta::assert_snapshot!(render_fragment!(details_failed().as_overview()));
    }

    #[test]
    fn trigger_form_snapshot() {
        insta::assert_snapshot!(render_fragment!(
            preview::test_plan_detail().as_trigger_form()
        ));
    }

    #[test]
    fn trigger_form_snapshot_with_error_and_variables() {
        insta::assert_snapshot!(render_fragment!(
            preview::test_plan_detail_trigger_error().as_trigger_form()
        ));
    }

    #[test]
    fn trigger_form_is_omitted_without_details() {
        let body = details_failed().as_trigger_form().render().unwrap();
        assert!(body.trim().is_empty(), "got: {body}");
    }

    #[test]
    fn runs_snapshot() {
        let uuid = Uuid::from_u128(10);
        let started_at = Utc.with_ymd_and_hms(2024, 3, 15, 12, 0, 0).unwrap();
        let response = TestRunListResponse {
            runs: vec![rtf_orchestrator_shared::summary::TestRunSummary {
                started_at,
                updated_at: started_at,
                ..mocks::sample_summary(Uuid::from_u128(1), Uuid::from_u128(2), Status::Successful)
            }],
            total: 1,
        };
        let t = TestPlanDetailTemplate {
            runs: Some(RunListView::for_known_test_plan(response, 20, 0, uuid)),
            ..preview::test_plan_detail()
        };
        insta::assert_snapshot!(render_fragment!(t.as_runs()));
    }

    #[test]
    fn runs_snapshot_runs_failed() {
        let t = TestPlanDetailTemplate {
            runs: None,
            runs_error: Some("Could not load recent runs for this test plan.".to_owned()),
            ..preview::test_plan_detail()
        };
        insta::assert_snapshot!(render_fragment!(t.as_runs()));
    }
}
