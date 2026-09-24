//! Canned data for every page template, built from the same `orchestrator::mocks` summary
//! builders the crate's own unit tests use.
//!
//! Used by `examples/preview`
use crate::{links::sample_config, orchestrator::mocks};
use chrono::Utc;
use rtf_orchestrator_shared::{
    known_test_plan::KnownTestPlanListResponse,
    status::Status,
    summary::{TestRunListResponse, TestRunSummary},
    test_plan_details::{DEFAULT_DAYS, DEFAULT_DAYS_BACK},
};
use serde_json::json;
use uuid::Uuid;

// Re-exported so `examples/preview` can name these types (and so it renders the exact same
// `Template`/view-model types the real handlers do, not lookalikes).
pub use crate::templates::*;
pub use crate::view::*;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub fn index() -> IndexTemplate {
    let response = TestRunListResponse {
        runs: vec![
            mocks::sample_summary(id(1), id(2), Status::Running),
            mocks::sample_summary(id(3), id(4), Status::Successful),
            mocks::sample_summary(id(5), id(6), Status::Failed),
            mocks::sample_summary(id(9), id(3), Status::Cancelled),
        ],
        total: 3,
    };

    IndexTemplate {
        initiated_by: String::new(),
        started_within: String::new(),
        list: Some(RunListView::new(
            response,
            20,
            0,
            String::new(),
            String::new(),
        )),
        list_error: None,
    }
}

pub fn run_running() -> RunTemplate {
    let run = TestRunSummary {
        test_plan_id: Some(id(10)),
        trigger_variables: Some(
            json!({"environment": "staging", "regions": ["us-east-1", "eu-west-1"]}),
        ),
        ..mocks::sample_summary(id(1), id(2), Status::Running)
    };

    RunTemplate {
        run: RunView::new(run, Utc::now(), &sample_config(), String::new()),
    }
}

pub fn run_terminal() -> RunTemplate {
    let now = Utc::now();
    let run = TestRunSummary {
        test_plan_id: Some(id(10)),
        completed_at: Some(now),
        ..mocks::sample_summary(id(3), id(4), Status::Successful)
    };

    RunTemplate {
        run: RunView::new(run, now, &sample_config(), String::new()),
    }
}

pub fn run_not_found() -> RunNotFoundTemplate {
    RunNotFoundTemplate {
        id: id(999).to_string(),
    }
}

pub fn execution_with_parent() -> ExecutionTemplate {
    ExecutionTemplate {
        execution: ExecutionDetailView::new(
            mocks::sample_execution(id(1), id(2)),
            &sample_config(),
        ),
    }
}

pub fn execution_not_found() -> ExecutionNotFoundTemplate {
    ExecutionNotFoundTemplate {
        execution_id: id(404).to_string(),
    }
}

pub fn test_plans() -> TestPlansTemplate {
    let response = KnownTestPlanListResponse {
        test_plans: vec![
            mocks::sample_known_test_plan(id(10)),
            mocks::sample_known_test_plan(id(11)),
        ],
        total: 2,
    };

    TestPlansTemplate {
        list: Some(KnownTestPlanListView::new(response, 20, 0)),
        list_error: None,
    }
}

pub fn test_plan_detail() -> TestPlanDetailTemplate {
    let uuid = id(10);
    let runs = TestRunListResponse {
        runs: vec![mocks::sample_summary(id(1), id(2), Status::Successful)],
        total: 1,
    };

    TestPlanDetailTemplate {
        plan: KnownTestPlanRowView::from(mocks::sample_known_test_plan(uuid)),
        details: Some(TestPlanDetailsView::new(
            mocks::sample_test_plan_details(uuid),
            DEFAULT_DAYS_BACK,
            DEFAULT_DAYS,
        )),
        details_error: None,
        runs: Some(RunListView::for_known_test_plan(runs, 20, 0, uuid)),
        runs_error: None,
        trigger_git_ref: String::new(),
        days_back: DEFAULT_DAYS_BACK,
        days: DEFAULT_DAYS,
        trigger_variables: String::new(),
        trigger_error: None,
    }
}

pub fn test_plan_detail_trigger_error() -> TestPlanDetailTemplate {
    let uuid = id(10);
    let runs = TestRunListResponse {
        runs: vec![mocks::sample_summary(id(1), id(2), Status::Successful)],
        total: 1,
    };

    TestPlanDetailTemplate {
        plan: KnownTestPlanRowView::from(mocks::sample_known_test_plan(uuid)),
        details: Some(TestPlanDetailsView::new(
            mocks::sample_test_plan_details(uuid),
            DEFAULT_DAYS_BACK,
            DEFAULT_DAYS,
        )),
        details_error: None,
        runs: Some(RunListView::for_known_test_plan(runs, 20, 0, uuid)),
        runs_error: None,
        trigger_git_ref: String::new(),
        days_back: DEFAULT_DAYS_BACK,
        days: DEFAULT_DAYS,
        trigger_variables: r#"{"region": "us-east-1",}"#.to_owned(),
        trigger_error: Some(
            "Variables must be a JSON object: trailing comma at line 1 column 24".to_owned(),
        ),
    }
}

pub fn test_plan_detail_k8s() -> TestPlanDetailTemplate {
    let uuid = id(10);
    let runs = TestRunListResponse {
        runs: vec![mocks::sample_summary(id(1), id(2), Status::Successful)],
        total: 1,
    };

    TestPlanDetailTemplate {
        plan: KnownTestPlanRowView::from(mocks::sample_known_test_plan(uuid)),
        details: Some(TestPlanDetailsView::new(
            mocks::sample_k8s_test_plan_details(uuid),
            DEFAULT_DAYS_BACK,
            DEFAULT_DAYS,
        )),
        details_error: None,
        runs: Some(RunListView::for_known_test_plan(runs, 20, 0, uuid)),
        runs_error: None,
        trigger_git_ref: String::new(),
        days_back: DEFAULT_DAYS_BACK,
        days: DEFAULT_DAYS,
        trigger_variables: String::new(),
        trigger_error: None,
    }
}

pub fn test_plan_not_found() -> TestPlanNotFoundTemplate {
    TestPlanNotFoundTemplate {
        uuid: id(404).to_string(),
    }
}

pub fn trigger_empty() -> TriggerTemplate {
    TriggerTemplate::default()
}

pub fn trigger_error() -> TriggerTemplate {
    TriggerTemplate {
        org: "apollographql".to_owned(),
        repo: "runtime-testing-framework".to_owned(),
        path: "test-plans/example.yaml".to_owned(),
        git_ref: "main".to_owned(),
        variables: r#"{"duration": "60s"}"#.to_owned(),
        error: Some("unknown path in repo".to_owned()),
    }
}

pub fn error_page() -> ErrorTemplate {
    ErrorTemplate {
        message: "the orchestrator returned an unexpected error".to_owned(),
    }
}
