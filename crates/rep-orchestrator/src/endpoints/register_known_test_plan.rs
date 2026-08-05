//! Register a Test Plan with the orchestrator so it can be triggered and queried by UUID or name.
//!
//! Gated to admins only (see [super::AdminUser]).
use crate::{Result, conn, db::KnownTestPlan, endpoints::AdminUser};
use axum::Json;
use rep_orchestrator_shared::known_test_plan::{KnownTestPlanSummary, RegisterTestPlanRequest};

pub async fn handler(
    _admin: AdminUser,
    Json(req): Json<RegisterTestPlanRequest>,
) -> Result<Json<KnownTestPlanSummary>> {
    let known = KnownTestPlan::register(
        &req.name,
        req.description.as_deref(),
        &req.org,
        &req.repo,
        &req.path,
        conn!(),
    )
    .await?;

    Ok(Json(known.into_summary()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{iap_identity::IAP_USER_EMAIL_HEADER, test_helpers::TestServerState};
    use reqwest::StatusCode;
    use uuid::Uuid;

    const ADMIN_EMAIL: &str = "admin@test.com";

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn admin_header_value() -> String {
        format!("accounts.google.com:{ADMIN_EMAIL}")
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn admin_can_register_a_known_test_plan() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let name = unique("registrable");
        let path = unique("path");

        let resp = tss
            .test_server
            .post("/test-plan/register")
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&RegisterTestPlanRequest {
                name: name.clone(),
                description: Some("a test plan".to_owned()),
                org: "my-org".to_owned(),
                repo: "my-repo".to_owned(),
                path: path.clone(),
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let summary: KnownTestPlanSummary = resp.json();
        assert_eq!(summary.name, name);
        assert_eq!(summary.description.as_deref(), Some("a test plan"));
        assert_eq!(summary.org, "my-org");
        assert_eq!(summary.repo, "my-repo");
        assert_eq!(summary.path, path);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn duplicate_name_is_rejected_with_conflict() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);

        let req = RegisterTestPlanRequest {
            name: unique("dup"),
            description: None,
            org: "my-org".to_owned(),
            repo: "my-repo".to_owned(),
            path: unique("path"),
        };

        let first = tss
            .test_server
            .post("/test-plan/register")
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&req)
            .await;
        assert_eq!(first.status_code(), StatusCode::OK);

        let second = tss
            .test_server
            .post("/test-plan/register")
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&req)
            .await;
        assert_eq!(second.status_code(), StatusCode::CONFLICT);

        Ok(())
    }

    // A user present in one server's admins list is not implicitly an admin on another; each
    // TestServerState carries its own fixed list rather than a shared/global one.
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn non_admin_is_rejected() -> anyhow::Result<()> {
        let tss = TestServerState::new();

        let resp = tss
            .test_server
            .post("/test-plan/register")
            .add_header(
                IAP_USER_EMAIL_HEADER,
                "accounts.google.com:someone@my-project.iam.gserviceaccount.com",
            )
            .json(&RegisterTestPlanRequest {
                name: unique("should-not-register"),
                description: None,
                org: "my-org".to_owned(),
                repo: "my-repo".to_owned(),
                path: "test-plans/example".to_owned(),
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn unauthenticated_caller_is_rejected() -> anyhow::Result<()> {
        let tss = TestServerState::new();

        let resp = tss
            .test_server
            .post("/test-plan/register")
            .json(&RegisterTestPlanRequest {
                name: unique("should-not-register"),
                description: None,
                org: "my-org".to_owned(),
                repo: "my-repo".to_owned(),
                path: "test-plans/example".to_owned(),
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }
}
