use crate::view::KnownTestPlanListView;
use askama::Template;

/// The known test plans page: a paginated table of test plans registered with the orchestrator.
#[derive(Template)]
#[template(path = "test_plans.html")]
pub struct TestPlansTemplate {
    pub list: Option<KnownTestPlanListView>,
    pub list_error: Option<String>,
}
