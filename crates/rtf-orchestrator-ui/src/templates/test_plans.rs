use crate::view::KnownTestPlanListView;
use askama::Template;

#[derive(Template)]
#[template(path = "test_plans.html", blocks = ["plans"])]
pub struct TestPlansTemplate {
    pub list: Option<KnownTestPlanListView>,
    pub list_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{orchestrator::mocks::sample_known_test_plan, render_fragment};
    use rtf_orchestrator_shared::known_test_plan::KnownTestPlanListResponse;
    use uuid::Uuid;

    fn test_plans(response: KnownTestPlanListResponse) -> TestPlansTemplate {
        TestPlansTemplate {
            list: Some(KnownTestPlanListView::new(response, 20, 0)),
            list_error: None,
        }
    }

    #[test]
    fn plans_snapshot() {
        let t = test_plans(KnownTestPlanListResponse {
            test_plans: vec![sample_known_test_plan(Uuid::from_u128(1))],
            total: 1,
        });
        insta::assert_snapshot!(render_fragment!(t.as_plans()));
    }

    #[test]
    fn plans_snapshot_empty() {
        let t = test_plans(KnownTestPlanListResponse::default());
        insta::assert_snapshot!(render_fragment!(t.as_plans()));
    }

    #[test]
    fn plans_snapshot_listing_failed() {
        let t = TestPlansTemplate {
            list: None,
            list_error: Some("Could not load known test plans from the orchestrator.".to_owned()),
        };
        insta::assert_snapshot!(render_fragment!(t.as_plans()));
    }
}
