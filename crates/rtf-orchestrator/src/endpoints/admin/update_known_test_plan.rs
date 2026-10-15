//! Update the stored details of a known test plan.
//!
//! Every field in the request body is optional: a field left out of the payload is left
//! unchanged. `description` and `pinned_workload_pool` additionally accept an explicit JSON
//! `null` to clear the stored value, unpinning the pool in the latter case.
//!
//! - Admins may update any field
//! - Owners may update a subset of fields (see [KnownTestPlan::validate_and_update])
use crate::{
    Error, Result, conn,
    db::KnownTestPlan,
    state::{KnownUser, ServerState},
};
use axum::{
    Json,
    extract::{Path, State},
};
use rtf_orchestrator_shared::known_test_plan::{KnownTestPlanSummary, UpdateKnownTestPlanRequest};
use uuid::Uuid;

pub async fn handler(
    user: KnownUser,
    Path(uuid): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
    Json(req): Json<UpdateKnownTestPlanRequest>,
) -> Result<Json<KnownTestPlanSummary>> {
    let conn = conn!();
    let known = KnownTestPlan::get_by_uuid(&uuid, conn)
        .await?
        .ok_or_else(|| Error::UnknownTestPlan {
            identifier: uuid.to_string(),
        })?;

    let updated = known
        .validate_and_update(
            &user,
            req,
            &eq_state.available_pools(),
            &eq_state.dedicated_pools(),
            conn,
        )
        .await?;

    Ok(Json(updated.try_into_summary(conn).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, DEFAULT_POOL, DEFAULT_POOL_NAME, NamedPoolConfig, PoolConfig},
        iap_identity::IAP_USER_EMAIL_HEADER,
        test_helpers::TestServerState,
    };
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::{known_test_plan::Role, workload_config::WorkloadConfigPatch};
    use sqlx::PgConnection;
    use std::collections::{BTreeMap, HashSet};

    const ADMIN_EMAIL: &str = "admin@test.com";
    const OWNER_EMAIL: &str = "owner@test.com";

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
    async fn admin_can_pin_and_unpin_a_configured_pool() -> anyhow::Result<()> {
        let mut cfg = Config::get().clone();
        let mut cluster_cfg = cfg.workload_clusters.available_clusters[0].clone();
        cluster_cfg.name = "beta".into();
        cfg.workload_clusters.available_clusters.push(cluster_cfg);
        cfg.workload_clusters
            .cluster_pools
            .additional
            .push(NamedPoolConfig {
                name: "perf".into(),
                config: PoolConfig {
                    supports_dedicated: false,
                    available_clusters: vec!["beta".into()],
                    per_user_limits: Default::default(),
                    workload_config: Default::default(),
                },
            });

        let tss = TestServerState::new_with_config_and_admins(&cfg, &[ADMIN_EMAIL]);

        let plan = register_plan(conn!()).await;
        assert_eq!(plan.pinned_workload_pool(), None);

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                pinned_workload_pool: Some(Some("perf".to_owned())),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let pinned: KnownTestPlanSummary = resp.json();
        assert_eq!(pinned.pinned_workload_pool.as_deref(), Some("perf"));

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                pinned_workload_pool: Some(None),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let unpinned: KnownTestPlanSummary = resp.json();
        assert_eq!(unpinned.pinned_workload_pool, None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn pin_to_unconfigured_pool_is_rejected() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&UpdateKnownTestPlanRequest {
                pinned_workload_pool: Some(Some("unknown".to_owned())),
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
        assert_eq!(fetched.pinned_workload_pool(), None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn admin_can_set_and_replace_the_workload_config_patch() -> anyhow::Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;
        assert_eq!(
            plan.workload_config_patch()?,
            WorkloadConfigPatch::default()
        );

        let resp = put_update(
            &tss,
            &plan,
            UpdateKnownTestPlanRequest {
                workload_config_patch: Some(WorkloadConfigPatch {
                    allow_k8s_write: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let updated: KnownTestPlanSummary = resp.json();
        assert_eq!(updated.workload_config_patch.allow_k8s_write, Some(true));

        let resp = put_update(
            &tss,
            &plan,
            UpdateKnownTestPlanRequest {
                workload_config_patch: Some(WorkloadConfigPatch::default()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let cleared: KnownTestPlanSummary = resp.json();
        assert_eq!(
            cleared.workload_config_patch,
            WorkloadConfigPatch::default()
        );

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
                pinned_workload_pool: Some(Some(DEFAULT_POOL.to_string())),
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
                pinned_workload_pool: Some(Some(DEFAULT_POOL.to_string())),
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
                pinned_workload_pool: Some(Some(DEFAULT_POOL.to_string())),
                ..Default::default()
            })
            .await;

        assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);

        Ok(())
    }

    async fn put_update(
        tss: &TestServerState,
        plan: &KnownTestPlan,
        req: UpdateKnownTestPlanRequest,
    ) -> axum_test::TestResponse {
        tss.test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(IAP_USER_EMAIL_HEADER, admin_header_value())
            .json(&req)
            .await
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn owner_can_update_a_permitted_field() -> Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!())
            .await
            .validate_and_update(
                &KnownUser::Admin(ADMIN_EMAIL.into()),
                UpdateKnownTestPlanRequest {
                    associated_users: Some(BTreeMap::from([(
                        OWNER_EMAIL.to_owned(),
                        Some(Role::Owner),
                    )])),
                    ..Default::default()
                },
                &HashSet::from([DEFAULT_POOL_NAME]),
                &HashSet::from([DEFAULT_POOL_NAME]),
                conn!(),
            )
            .await?;

        let resp = tss
            .test_server
            .put(&format!("/admin/test-plan/{}", plan.uuid()))
            .add_header(
                IAP_USER_EMAIL_HEADER,
                format!("accounts.google.com:{OWNER_EMAIL}"),
            )
            .json(&UpdateKnownTestPlanRequest {
                description: Some(Some("updated by owner".to_owned())),
                ..Default::default()
            })
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{:?}", resp.text());

        let updated: KnownTestPlanSummary = resp.json();
        assert_eq!(updated.description.as_deref(), Some("updated by owner"));

        Ok(())
    }

    fn requires_dedicated(value: bool) -> UpdateKnownTestPlanRequest {
        UpdateKnownTestPlanRequest {
            requires_dedicated_cluster: Some(value),
            ..Default::default()
        }
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn requires_dedicated_cluster_is_rejected_on_a_pool_without_support() -> anyhow::Result<()>
    {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan(conn!()).await;

        let resp = put_update(&tss, &plan, requires_dedicated(true)).await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
        let fetched = KnownTestPlan::get_by_uuid(&plan.uuid(), conn!())
            .await?
            .expect("plan should still exist");
        assert!(!fetched.requires_dedicated_cluster());

        Ok(())
    }
}
