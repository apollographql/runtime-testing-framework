//! Manage the named variables registered against a known test plan.
//!
//! Only admins and owners of the test plan are permitted to modify its registered variables.
use crate::{Error, Result, conn, db::KnownTestPlan, state::KnownUser};
use axum::{Json, extract::Path, http::StatusCode};
use rtf_core::variables::VariableOverride;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn put_handler(
    user: KnownUser,
    Path((uuid, name)): Path<(Uuid, String)>,
    Json(variables): Json<HashMap<String, VariableOverride>>,
) -> Result<StatusCode> {
    let conn = conn!();
    let ktp = test_plan_if_allowed(&user, uuid, conn).await?;
    ktp.add_variables(&name, &variables, conn).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_handler(
    user: KnownUser,
    Path((uuid, name)): Path<(Uuid, String)>,
) -> Result<StatusCode> {
    let conn = conn!();
    let ktp = test_plan_if_allowed(&user, uuid, conn).await?;
    ktp.remove_variables(&name, conn).await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn test_plan_if_allowed(
    user: &KnownUser,
    uuid: Uuid,
    conn: &mut PgConnection,
) -> Result<KnownTestPlan> {
    let ktp = KnownTestPlan::get_by_uuid(&uuid, conn)
        .await?
        .ok_or_else(|| Error::UnknownTestPlan {
            identifier: uuid.to_string(),
        })?;

    match user {
        KnownUser::Admin(_) => Ok(ktp),
        KnownUser::User(email) => {
            if ktp.is_owner(email, conn).await? {
                Ok(ktp)
            } else {
                Err(Error::Unauthorized)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::DEFAULT_POOL_NAME, iap_identity::IAP_USER_EMAIL_HEADER,
        test_helpers::TestServerState,
    };
    use reqwest::Method;
    use rtf_orchestrator_shared::known_test_plan::{Role, UpdateKnownTestPlanRequest};
    use serde_json::json;
    use simple_test_case::test_case;
    use std::collections::{BTreeMap, HashSet};

    const ADMIN_EMAIL: &str = "admin@test.com";
    const OWNER_EMAIL: &str = "owner@test.com";
    const TRIGGER_EMAIL: &str = "trigger@test.com";
    const OTHER_EMAIL: &str = "other@test.com";

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn header(email: &str) -> String {
        format!("accounts.google.com:{email}")
    }

    async fn register_plan_with_users() -> Result<KnownTestPlan> {
        let plan = KnownTestPlan::register(
            &unique("plan"),
            None,
            "org",
            "repo",
            &unique("path"),
            conn!(),
        )
        .await?
        .validate_and_update(
            &KnownUser::Admin(ADMIN_EMAIL.into()),
            UpdateKnownTestPlanRequest {
                associated_users: Some(BTreeMap::from([
                    (OWNER_EMAIL.to_owned(), Some(Role::Owner)),
                    (TRIGGER_EMAIL.to_owned(), Some(Role::CanTrigger)),
                ])),
                ..Default::default()
            },
            &HashSet::from([DEFAULT_POOL_NAME]),
            &HashSet::from([DEFAULT_POOL_NAME]),
            conn!(),
        )
        .await?;

        Ok(plan)
    }

    fn graph_ref_vars() -> HashMap<String, VariableOverride> {
        HashMap::from([(
            "graph_ref".to_string(),
            VariableOverride::Scalar("graph@current".into()),
        )])
    }

    async fn do_request(
        email: &str,
        method: Method,
    ) -> Result<(StatusCode, Option<HashMap<String, VariableOverride>>)> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);
        let plan = register_plan_with_users().await?;
        let vars = graph_ref_vars();

        if method == Method::DELETE {
            plan.add_variables("vars", &vars, conn!()).await?;
        }

        let mut req = tss
            .test_server
            .method(
                method.clone(),
                &format!("/admin/test-plan/{}/variables/vars", plan.uuid()),
            )
            .add_header(IAP_USER_EMAIL_HEADER, header(email));

        if method == Method::PUT {
            req = req.json(&vars);
        }

        let resp = req.await;
        let vars = plan.variables_by_name("vars", conn!()).await?;

        Ok((resp.status_code(), vars))
    }

    #[test_case(ADMIN_EMAIL; "admin")]
    #[test_case(OWNER_EMAIL; "owner")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn put_permitted_user_succeeds(email: &str) {
        let (status_code, vars) = do_request(email, Method::PUT).await.unwrap();

        // request should succeed, variables should be stored
        assert_eq!(status_code, StatusCode::NO_CONTENT);
        assert_eq!(vars, Some(graph_ref_vars()));
    }

    #[test_case(TRIGGER_EMAIL; "trigger user")]
    #[test_case(OTHER_EMAIL; "unassociated user")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn put_other_callers_are_rejected(email: &str) {
        let (status_code, vars) = do_request(email, Method::PUT).await.unwrap();

        // request should fail, variables should NOT be stored
        assert_eq!(status_code, StatusCode::FORBIDDEN);
        assert_eq!(vars, None);
    }

    #[test_case(ADMIN_EMAIL; "admin")]
    #[test_case(OWNER_EMAIL; "owner")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn delete_permitted_user_succeeds(email: &str) {
        let (status_code, vars) = do_request(email, Method::DELETE).await.unwrap();

        // request should succeed, variables should be removed
        assert_eq!(status_code, StatusCode::NO_CONTENT);
        assert_eq!(vars, None);
    }

    #[test_case(TRIGGER_EMAIL; "trigger user")]
    #[test_case(OTHER_EMAIL; "unassociated user")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn delete_other_callers_are_rejected(email: &str) {
        let (status_code, vars) = do_request(email, Method::DELETE).await.unwrap();

        // request should fail, variables should NOT be removed
        assert_eq!(status_code, StatusCode::FORBIDDEN);
        assert_eq!(vars, Some(graph_ref_vars()));
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn unknown_plan_returns_404() -> Result<()> {
        let tss = TestServerState::new_with_admins(&[ADMIN_EMAIL]);

        let resp = tss
            .test_server
            .put(&format!(
                "/admin/test-plan/{}/variables/vars",
                Uuid::new_v4()
            ))
            .add_header(IAP_USER_EMAIL_HEADER, header(ADMIN_EMAIL))
            .json(&json!({ "foo": 1 }))
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }
}
