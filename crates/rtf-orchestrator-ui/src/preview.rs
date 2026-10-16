//! Canned data for every page template, used by `examples/preview`.
use crate::{links::sample_config, orchestrator::mocks};
use chrono::Utc;
use rtf_orchestrator_shared::{
    known_test_plan::{KnownTestPlanListResponse, KnownTestPlanSummary, Role},
    status::Status,
    summary::{TestRunListResponse, TestRunSummary},
    test_plan_details::{DEFAULT_DAYS, DEFAULT_DAYS_BACK, TestPlanDetails},
    whoami::WhoAmIResponse,
};
use serde_json::json;
use std::collections::BTreeMap;
use uuid::Uuid;

pub use crate::templates::*;
pub use crate::view::*;

pub const OWNER_EMAIL: &str = "owner@apollographql.com";
pub const TRIGGER_USER_EMAIL: &str = "trigger-user@apollographql.com";

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub fn index() -> IndexTemplate {
    let response = TestRunListResponse {
        runs: vec![
            mocks::sample_summary(id(1), id(2), Status::Running),
            mocks::sample_summary_with_initiator(
                id(3),
                id(4),
                Status::Successful,
                "automation:release-tooling-rtf@runtime-testing-framework.iam.gserviceaccount.com:ORG:REPO",
            ),
            mocks::sample_summary_with_initiator(id(5), id(6), Status::Failed, "unknown"),
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
        dashboard: Some(DashboardView::new(
            mocks::sample_event_queue_snapshot(),
            mocks::sample_cluster_summary(),
            sample_config().cluster_roles_url,
        )),
        dashboard_error: None,
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
            KnownTestPlanSummary {
                associated_users: BTreeMap::from([(OWNER_EMAIL.to_owned(), Role::CanTrigger)]),
                restrict_automated_triggers: false,
                ..mocks::sample_known_test_plan(id(11))
            },
            KnownTestPlanSummary {
                associated_users: BTreeMap::new(),
                restrict_triggers: false,
                restrict_automated_triggers: false,
                ..mocks::sample_known_test_plan(id(12))
            },
        ],
        total: 3,
    };

    TestPlansTemplate {
        list: Some(KnownTestPlanListView::new(
            response,
            &user(OWNER_EMAIL),
            20,
            0,
        )),
        list_error: None,
    }
}

fn test_plan_detail_for(
    whoami: WhoAmIResponse,
    plan: KnownTestPlanSummary,
    details: TestPlanDetails,
) -> TestPlanDetailTemplate {
    let uuid = plan.uuid;
    let runs = TestRunListResponse {
        runs: vec![mocks::sample_summary(id(1), id(2), Status::Successful)],
        total: 1,
    };
    let details = TestPlanDetails {
        pool: plan.pinned_workload_pool.clone().unwrap_or(details.pool),
        ..details
    };

    TestPlanDetailTemplate {
        badge: ViewerBadge::for_test_plan(&whoami, &plan),
        can_trigger: whoami.can_trigger(&plan),
        plan: KnownTestPlanRowView::from(plan),
        details: Some(TestPlanDetailsView::new(
            details,
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

pub fn user(email: &str) -> WhoAmIResponse {
    WhoAmIResponse::User {
        email: email.to_owned(),
    }
}

fn admin() -> WhoAmIResponse {
    WhoAmIResponse::Admin {
        email: "admin@apollographql.com".to_owned(),
    }
}

pub fn test_plan_detail_for_user(whoami: WhoAmIResponse) -> TestPlanDetailTemplate {
    test_plan_detail_for(
        whoami,
        mocks::sample_known_test_plan(id(10)),
        mocks::sample_test_plan_details(id(10)),
    )
}

pub fn test_plan_detail() -> TestPlanDetailTemplate {
    test_plan_detail_for_user(admin())
}

pub fn test_plan_detail_trigger_error() -> TestPlanDetailTemplate {
    TestPlanDetailTemplate {
        trigger_variables: r#"{"region": "us-east-1",}"#.to_owned(),
        trigger_error: Some(
            "Variables must be a JSON object: trailing comma at line 1 column 24".to_owned(),
        ),
        ..test_plan_detail()
    }
}

pub fn test_plan_detail_k8s() -> TestPlanDetailTemplate {
    test_plan_detail_for(
        admin(),
        mocks::sample_known_test_plan(id(10)),
        mocks::sample_k8s_test_plan_details(id(10)),
    )
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

pub fn viewer_badge_admin() -> ViewerBadgeTemplate {
    ViewerBadgeTemplate {
        badge: ViewerBadge::for_whoami(&WhoAmIResponse::Admin {
            email: "admin@apollographql.com".to_owned(),
        }),
    }
}

pub fn error_page() -> ErrorTemplate {
    ErrorTemplate {
        message: "the orchestrator returned an unexpected error".to_owned(),
    }
}
