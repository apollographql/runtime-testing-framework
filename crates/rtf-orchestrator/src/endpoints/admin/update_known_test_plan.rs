//! Update the stored details of a known test plan.
//!
//! Every field in the request body is optional: a field left out of the payload is left
//! unchanged. `description` and `pinned_workload_cluster` additionally accept an explicit JSON
//! `null` to clear the stored value, unpinning the cluster in the latter case.
//!
//! Gated to admins only (see [AdminUser]).
use crate::{
    Error, Result, conn,
    db::{ClusterId, KnownTestPlan},
    endpoints::AdminUser,
    state::ServerState,
};
use axum::{
    Json,
    extract::{Path, State},
};
use rtf_orchestrator_shared::known_test_plan::{KnownTestPlanSummary, UpdateKnownTestPlanRequest};
use uuid::Uuid;

pub async fn handler(
    _admin: AdminUser,
    Path(uuid): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
    Json(req): Json<UpdateKnownTestPlanRequest>,
) -> Result<Json<KnownTestPlanSummary>> {
    if let Some(Some(cluster)) = &req.pinned_cluster {
        let requested = ClusterId::new(cluster);
        if !eq_state.available_clusters().await.contains(&requested) {
            return Err(Error::UnknownWorkloadCluster {
                cluster: cluster.clone(),
            });
        }
    }

    let conn = conn!();
    let known = KnownTestPlan::get_by_uuid(&uuid, conn)
        .await?
        .ok_or_else(|| Error::UnknownTestPlan {
            identifier: uuid.to_string(),
        })?;

    let updated = known.update(req, conn).await?;

    Ok(Json(updated.into_summary()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config, iap_identity::IAP_USER_EMAIL_HEADER, test_helpers::TestServerState,
    };
    use reqwest::StatusCode;
    use sqlx::PgConnection;

    const ADMIN_EMAIL: &str = "admin@test.com";

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn admin_header_value() -> String {
        format!("accounts.google.com:{ADMIN_EMAIL}")
    }

    async fn register_plan(conn: &mut PgConnection) -> KnownTestPlan {
        KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), conn)
            .await
            .unwrap()
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn admin_can_pin_and_unpin_a_configured_cluster() -> anyhow::Result<()> {
        let mut cfg = Config::get().clone();
        let mut cluster_cfg = cfg.workload_clusters.available_clusters[0].clone();
        cluster_cfg.name = "beta".into();
        cfg.workload_clusters.available_clusters.push(cluster_cfg);

        let tss = TestServerState::new_with_config_and_admins(&cfg, &[ADMIN_EMAIL]);

        let plan = register_plan(conn!()).await;
        assert_eq!(plan.pinned_workload_cluster(), None);

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                pinned_cluster: Some(Some("beta".to_owned())),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let pinned: KnownTestPlanSummary = resp.json();
        assert_eq!(pinned.pinned_workload_cluster.as_deref(), Some("beta"));

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                pinned_cluster: Some(None),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let unpinned: KnownTestPlanSummary = resp.json();
        assert_eq!(unpinned.pinned_workload_cluster, None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn pin_to_unconfigured_cluster_is_rejected() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                pinned_cluster: Some(Some("unknown".to_owned())),
                ..Default::default()
            })
            .await;

        assert_eq!(
            resp.status_code(),
            StatusCode::BAD_REQUEST,
            "{:?}",
            resp.text()
        );

        let fetched = KnownTestPlan::get_by_uuid(&plan.uuid(), conn!())
            .await?
            .expect("plan should still exist");
        assert_eq!(fetched.pinned_workload_cluster(), None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn admin_can_set_and_clear_the_k8s_write_flag() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;
        assert!(!plan.allow_k8s_write());

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                allow_k8s_write: Some(true),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let updated: KnownTestPlanSummary = resp.json();
        assert!(updated.allow_k8s_write);

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                allow_k8s_write: Some(false),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let cleared: KnownTestPlanSummary = resp.json();
        assert!(!cleared.allow_k8s_write);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn admin_can_update_the_plan_details() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;
        let new_name = unique("renamed");
        let new_path = unique("path");

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                name: Some(new_name.clone()),
                description: Some(Some("updated description".to_owned())),
                org: Some("new-org".to_owned()),
                repo: Some("new-repo".to_owned()),
                path: Some(new_path.clone()),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let updated: KnownTestPlanSummary = resp.json();
        assert_eq!(updated.name, new_name);
        assert_eq!(updated.description.as_deref(), Some("updated description"));
        assert_eq!(updated.org, "new-org");
        assert_eq!(updated.repo, "new-repo");
        assert_eq!(updated.path, new_path);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn updating_to_a_duplicate_name_is_rejected_with_conflict() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let existing = register_plan(conn!()).await;
        let plan = register_plan(conn!()).await;

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                name: Some(existing.name().to_owned()),
                ..Default::default()
            })
            .await;

        assert_eq!(
            resp.status_code(),
            StatusCode::CONFLICT,
            "{:?}",
            resp.text()
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn omitted_fields_are_left_unchanged() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest::default())
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let untouched: KnownTestPlanSummary = resp.json();
        assert_eq!(untouched.name, plan.name());
        assert_eq!(untouched.org, plan.org());
        assert_eq!(untouched.repo, plan.repo());
        assert_eq!(untouched.path, plan.path());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn unknown_plan_returns_404() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", Uuid::new_v4()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                pinned_cluster: Some(Some("alpha".to_owned())),
                ..Default::default()
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn non_admin_is_rejected() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(
                IAP_USER_EMAIL_HEADER,
                "accounts.google.com:someone@my-project.iam.gserviceaccount.com",
            )
            .json(&UpdateKnownTestPlanRequest {
                pinned_cluster: Some(Some("alpha".to_owned())),
                ..Default::default()
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn unauthenticated_caller_is_rejected() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .json(&UpdateKnownTestPlanRequest {
                pinned_cluster: Some(Some("alpha".to_owned())),
                ..Default::default()
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }
}
