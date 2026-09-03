//! Pin or unpin the workload cluster a known test plan's runs execute in.
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
use rtf_orchestrator_shared::known_test_plan::{
    KnownTestPlanSummary, SetPinnedWorkloadClusterRequest,
};
use uuid::Uuid;

pub async fn set_handler(
    _admin: AdminUser,
    Path(uuid): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
    Json(req): Json<SetPinnedWorkloadClusterRequest>,
) -> Result<Json<KnownTestPlanSummary>> {
    let conn = conn!();

    let requested = ClusterId::new(&req.cluster);
    if !eq_state.available_clusters().await.contains(&requested) {
        return Err(Error::UnknownWorkloadCluster {
            cluster: req.cluster,
        });
    }

    let mut known = KnownTestPlan::get_by_uuid(&uuid, conn)
        .await?
        .ok_or_else(|| Error::UnknownTestPlan {
            identifier: uuid.to_string(),
        })?;

    known
        .set_pinned_workload_cluster(Some(&req.cluster), conn)
        .await?;

    Ok(Json(known.into_summary()))
}

pub async fn clear_handler(
    _admin: AdminUser,
    Path(uuid): Path<Uuid>,
) -> Result<Json<KnownTestPlanSummary>> {
    let conn = conn!();

    let mut known = KnownTestPlan::get_by_uuid(&uuid, conn)
        .await?
        .ok_or_else(|| Error::UnknownTestPlan {
            identifier: uuid.to_string(),
        })?;

    known.set_pinned_workload_cluster(None, conn).await?;

    Ok(Json(known.into_summary()))
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
            .post(&format!("/admin/test-plan/{}/pinned-cluster", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&SetPinnedWorkloadClusterRequest {
                cluster: "beta".to_owned(),
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let pinned: KnownTestPlanSummary = resp.json();
        assert_eq!(pinned.pinned_workload_cluster.as_deref(), Some("beta"));

        let resp = tss
            .test_server
            .delete(&format!("/admin/test-plan/{}/pinned-cluster", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
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
            .post(&format!("/admin/test-plan/{}/pinned-cluster", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&SetPinnedWorkloadClusterRequest {
                cluster: "unknown".to_owned(),
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
    async fn unknown_plan_returns_404() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);

        let resp = tss
            .test_server
            .post(&format!(
                "/admin/test-plan/{}/pinned-cluster",
                Uuid::new_v4()
            ))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&SetPinnedWorkloadClusterRequest {
                cluster: "alpha".to_owned(),
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
            .post(&format!("/admin/test-plan/{}/pinned-cluster", plan.uuid()))
            .add_header(
                IAP_USER_EMAIL_HEADER,
                "accounts.google.com:someone@my-project.iam.gserviceaccount.com",
            )
            .json(&SetPinnedWorkloadClusterRequest {
                cluster: "alpha".to_owned(),
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
            .post(&format!("/admin/test-plan/{}/pinned-cluster", plan.uuid()))
            .add_header(
                IAP_USER_EMAIL_HEADER,
                "accounts.google.com:someone@my-project.iam.gserviceaccount.com",
            )
            .json(&SetPinnedWorkloadClusterRequest {
                cluster: "alpha".to_owned(),
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }
}
