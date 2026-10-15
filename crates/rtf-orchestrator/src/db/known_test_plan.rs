use crate::{
    config::DEFAULT_POOL_NAME,
    db::{self, PoolId, Queryable, Result},
    state::KnownUser,
};
use rtf_orchestrator_shared::{
    known_test_plan::{KnownTestPlanSummary, Role as SharedRole, UpdateKnownTestPlanRequest},
    workload_config::WorkloadConfigPatch,
};
use serde_json::Value;
use sqlx::{Connection, PgConnection, Postgres, QueryBuilder};
use std::collections::{BTreeMap, HashSet};
use tracing::warn;
use uuid::Uuid;

/// A Test Plan registered with the orchestrator, identifiable by UUID or name so it can be
/// triggered (and its runs queried) without the caller needing to know its GitHub location.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct KnownTestPlan {
    id: i32,
    uuid: Uuid,
    name: String,
    description: Option<String>,
    org: String,
    repo: String,
    path: String,
    pinned_workload_pool: Option<String>,
    workload_config_patch: Option<Value>,
    requires_dedicated_cluster: bool,
    restrict_triggers: bool,
    restrict_automated_triggers: bool,
}

impl Queryable for KnownTestPlan {
    const TABLE_NAME: &'static str = "known_test_plan";

    fn id(&self) -> i32 {
        self.id
    }
}

impl KnownTestPlan {
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> Option<String> {
        self.description.clone()
    }

    pub fn org(&self) -> &str {
        &self.org
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn pinned_workload_pool(&self) -> Option<PoolId> {
        self.pinned_workload_pool.as_ref().map(PoolId::new)
    }

    pub fn workload_config_patch(&self) -> Result<WorkloadConfigPatch> {
        match &self.workload_config_patch {
            Some(raw) => {
                serde_json::from_value(raw.clone()).map_err(db::Error::MalformedWorkloadConfigPatch)
            }
            None => Ok(WorkloadConfigPatch::default()),
        }
    }

    pub fn requires_dedicated_cluster(&self) -> bool {
        self.requires_dedicated_cluster
    }

    /// Register a new known test plan.
    ///
    /// `name` and `(org, repo, path)` are each `UNIQUE` in the DB: attempting to register a
    /// duplicate of either surfaces as [db::Error::KnownTestPlanAlreadyExists] rather than a raw
    /// constraint violation.
    pub async fn register(
        name: &str,
        description: Option<&str>,
        org: &str,
        repo: &str,
        path: &str,
        conn: &mut PgConnection,
    ) -> Result<Self> {
        let res = sqlx::query_as(
            r#"
            INSERT INTO known_test_plan
              (name, description, org, repo, path)
            VALUES
              ($1, $2, $3, $4, $5)
            RETURNING
              id, uuid, name, description, org, repo, path, pinned_workload_pool,
              workload_config_patch, requires_dedicated_cluster, restrict_triggers,
              restrict_automated_triggers;
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(org)
        .bind(repo)
        .bind(path)
        .fetch_one(conn)
        .await;

        match res {
            Ok(known) => Ok(known),
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                Err(db::Error::KnownTestPlanAlreadyExists)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub async fn validate_and_update(
        self,
        user: &KnownUser,
        req: UpdateKnownTestPlanRequest,
        available_pools: &HashSet<&str>,
        dedicated_pools: &HashSet<&str>,
        conn: &mut PgConnection,
    ) -> crate::Result<Self> {
        self.validate_update_request(user, &req, available_pools, dedicated_pools, conn)
            .await?;

        Ok(self.update(req, conn).await?)
    }

    async fn validate_update_request(
        &self,
        user: &KnownUser,
        req: &UpdateKnownTestPlanRequest,
        available_pools: &HashSet<&str>,
        dedicated_pools: &HashSet<&str>,
        conn: &mut PgConnection,
    ) -> crate::Result<()> {
        self.check_update_permitted(user, req, conn).await?;

        if let Some(Some(pool)) = &req.pinned_workload_pool
            && !available_pools.contains(pool.as_str())
        {
            return Err(crate::Error::UnknownWorkloadPool { pool: pool.clone() });
        }

        let known_pinned = self.pinned_workload_pool();
        let pinned = match &req.pinned_workload_pool {
            Some(Some(pool)) => Some(pool.as_str()),
            Some(None) => None,
            None => known_pinned.as_ref().map(|p| p.as_str()),
        };
        let requires_dedicated = req
            .requires_dedicated_cluster
            .unwrap_or_else(|| self.requires_dedicated_cluster());

        let patch = match &req.workload_config_patch {
            Some(patch) => patch.clone(),
            None => self.workload_config_patch()?,
        };

        patch.validate(
            pinned.unwrap_or(DEFAULT_POOL_NAME),
            pinned.is_some(),
            requires_dedicated,
            dedicated_pools,
        )?;

        Ok(())
    }

    async fn check_update_permitted(
        &self,
        user: &KnownUser,
        req: &UpdateKnownTestPlanRequest,
        conn: &mut PgConnection,
    ) -> crate::Result<()> {
        // Admins can change anything
        let email = match user {
            KnownUser::Admin(_) => return Ok(()),
            KnownUser::User(email) => email,
        };

        // Non-owners can change nothing
        if !self.is_owner(email, conn).await? {
            return Err(crate::Error::Unauthorized);
        }

        // Owners can change anything other than the following fields
        if req.pinned_workload_pool.is_some() {
            return Err(crate::Error::AdminOnlyField {
                field: "pinned_workload_pool",
            });
        }

        if req.requires_dedicated_cluster.is_some() {
            return Err(crate::Error::AdminOnlyField {
                field: "requires_dedicated_cluster",
            });
        }

        // Attempting to remove yourself as an owner is not allowed
        let owners: HashSet<String> = self
            .associated_users(conn)
            .await?
            .into_iter()
            .filter_map(|(k, v)| if v == Role::Owner { Some(k) } else { None })
            .collect();

        let modifies_owners = req.associated_users.as_ref().is_some_and(|users| {
            users
                .iter()
                .any(|(k, v)| owners.contains(k) || *v == Some(SharedRole::Owner))
        });

        if modifies_owners {
            return Err(crate::Error::AdminOnlyField {
                field: "associated_users (Owner)",
            });
        }

        Ok(())
    }

    async fn update(
        self,
        update: UpdateKnownTestPlanRequest,
        conn: &mut PgConnection,
    ) -> Result<Self> {
        let mut tx = conn.begin().await?;

        if let Some(user_update) = update.associated_users.as_ref() {
            for (email, role) in user_update.iter() {
                self.update_associated_user(email, role.map(Role::from), &mut tx)
                    .await?;
            }
        }

        let updated = self.update_row(update, &mut tx).await?;
        tx.commit().await?;

        Ok(updated)
    }

    async fn update_associated_user(
        &self,
        email: &str,
        role: Option<Role>,
        conn: &mut PgConnection,
    ) -> Result<()> {
        let role = match role {
            Some(role) => role,
            None => return self.remove_associated_user(email, conn).await,
        };

        let (user_id,): (i32,) = sqlx::query_as(
            "INSERT INTO test_plan_user (email)
             VALUES ($1)
             ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email
             RETURNING id;",
        )
        .bind(email)
        .fetch_one(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO known_test_plan_user (user_id, known_test_plan_id, role)
             VALUES ($1, $2, $3)
             ON CONFLICT (known_test_plan_id, user_id)
             DO UPDATE SET role = EXCLUDED.role;",
        )
        .bind(user_id)
        .bind(self.id)
        .bind(role)
        .execute(conn)
        .await?;

        Ok(())
    }

    async fn remove_associated_user(&self, email: &str, conn: &mut PgConnection) -> Result<()> {
        sqlx::query(
            "DELETE FROM
               known_test_plan_user AS ktpu
             USING
               test_plan_user AS u
             WHERE
               ktpu.user_id = u.id
             AND
               ktpu.known_test_plan_id = $1
             AND
               u.email = $2;",
        )
        .bind(self.id)
        .bind(email)
        .execute(conn)
        .await?;

        Ok(())
    }

    async fn update_row(
        self,
        update: UpdateKnownTestPlanRequest,
        conn: &mut PgConnection,
    ) -> Result<Self> {
        if update.is_empty_row_update() {
            return Ok(self);
        }

        macro_rules! push_if_some {
            ($sep:expr, $opt:expr, $s:expr) => {
                if let Some(val) = $opt {
                    $sep.push($s);
                    $sep.push_bind_unseparated(val);
                }
            };
        }

        let mut qb = QueryBuilder::new("UPDATE known_test_plan SET ");
        {
            let mut sep = qb.separated(", ");
            push_if_some!(sep, update.name, "name = ");
            push_if_some!(sep, update.description, "description = ");
            push_if_some!(sep, update.org, "org = ");
            push_if_some!(sep, update.repo, "repo = ");
            push_if_some!(sep, update.path, "path = ");
            push_if_some!(sep, update.pinned_workload_pool, "pinned_workload_pool = ");
            push_if_some!(
                sep,
                update.workload_config_patch.map(|patch| {
                    serde_json::to_value(patch).expect("workload config patch to serialize")
                }),
                "workload_config_patch = "
            );
            push_if_some!(
                sep,
                update.requires_dedicated_cluster,
                "requires_dedicated_cluster = "
            );
            push_if_some!(sep, update.restrict_triggers, "restrict_triggers = ");
            push_if_some!(
                sep,
                update.restrict_automated_triggers,
                "restrict_automated_triggers = "
            );
        }
        qb.push(" WHERE id = ").push_bind(self.id);

        match qb.build().execute(&mut *conn).await {
            Ok(_) => {}
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                return Err(db::Error::KnownTestPlanAlreadyExists);
            }
            Err(e) => return Err(e.into()),
        }

        Self::get_by_id_unchecked(self.id, conn).await
    }

    pub async fn get_by_uuid(uuid: &Uuid, conn: &mut PgConnection) -> Result<Option<Self>> {
        Ok(
            sqlx::query_as("SELECT * FROM known_test_plan WHERE uuid = $1;")
                .bind(uuid)
                .fetch_optional(conn)
                .await?,
        )
    }

    pub async fn get_by_name(name: &str, conn: &mut PgConnection) -> Result<Option<Self>> {
        Ok(
            sqlx::query_as("SELECT * FROM known_test_plan WHERE name = $1;")
                .bind(name)
                .fetch_optional(conn)
                .await?,
        )
    }

    pub async fn is_owner(&self, email: &str, conn: &mut PgConnection) -> Result<bool> {
        Ok(self.associated_users(conn).await?.get(email) == Some(&Role::Owner))
    }

    pub async fn associated_users(
        &self,
        conn: &mut PgConnection,
    ) -> Result<BTreeMap<String, Role>> {
        let rows: Vec<(String, Role)> = sqlx::query_as(
            r#"
                SELECT
                  u.email, ktpu.role
                FROM
                  test_plan_user AS u
                JOIN known_test_plan_user AS ktpu
                  ON ktpu.user_id = u.id
                WHERE
                  ktpu.known_test_plan_id = $1
                ORDER BY
                  u.email;
                "#,
        )
        .bind(self.id())
        .fetch_all(conn)
        .await?;

        Ok(rows.into_iter().collect())
    }

    pub async fn try_into_summary(self, conn: &mut PgConnection) -> Result<KnownTestPlanSummary> {
        let workload_config_patch = self.workload_config_patch().unwrap_or_else(|e| {
            warn!(plan = %self.uuid, "{e}");
            WorkloadConfigPatch::default()
        });

        let associated_users = self
            .associated_users(conn)
            .await?
            .into_iter()
            .map(|(email, role)| (email, role.into()))
            .collect();

        Ok(KnownTestPlanSummary {
            uuid: self.uuid,
            name: self.name,
            description: self.description,
            org: self.org,
            repo: self.repo,
            path: self.path,
            pinned_workload_pool: self.pinned_workload_pool,
            workload_config_patch,
            requires_dedicated_cluster: self.requires_dedicated_cluster,
            associated_users,
            restrict_triggers: self.restrict_triggers,
            restrict_automated_triggers: self.restrict_automated_triggers,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, sqlx::Type)]
#[repr(i32)]
pub enum Role {
    Owner = 1,
    CanTrigger = 2,
}

impl From<Role> for SharedRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Owner => SharedRole::Owner,
            Role::CanTrigger => SharedRole::CanTrigger,
        }
    }
}

impl From<SharedRole> for Role {
    fn from(role: SharedRole) -> Self {
        match role {
            SharedRole::Owner => Role::Owner,
            SharedRole::CanTrigger => Role::CanTrigger,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct KnownTestPlanRun {
    id: i32,
    known_test_plan_id: i32,
    test_run_id: i32,
    git_sha: Option<String>,
}

impl Queryable for KnownTestPlanRun {
    const TABLE_NAME: &'static str = "known_test_plan_run";

    fn id(&self) -> i32 {
        self.id
    }
}

impl KnownTestPlanRun {
    pub async fn link(
        known_test_plan_id: i32,
        test_run_id: i32,
        git_sha: Option<&str>,
        conn: &mut PgConnection,
    ) -> Result<Self> {
        Ok(sqlx::query_as(
            "INSERT INTO known_test_plan_run (known_test_plan_id, test_run_id, git_sha)
             VALUES ($1, $2, $3)
             RETURNING id, known_test_plan_id, test_run_id, git_sha;
            ",
        )
        .bind(known_test_plan_id)
        .bind(test_run_id)
        .bind(git_sha)
        .fetch_one(conn)
        .await?)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownTestPlanFilter {
    pub name: Option<String>,
}

impl KnownTestPlanFilter {
    fn push_where_clause(&self, qb: &mut QueryBuilder<Postgres>) {
        if self.name.is_none() {
            return;
        }

        qb.push(" WHERE TRUE");

        if let Some(name) = &self.name {
            qb.push(" AND name = ").push_bind(name.clone());
        }
    }

    pub async fn matching(
        &self,
        limit: i64,
        offset: i64,
        conn: &mut PgConnection,
    ) -> Result<Vec<KnownTestPlan>> {
        let mut qb = QueryBuilder::new("SELECT * FROM known_test_plan");
        self.push_where_clause(&mut qb);

        qb.push(" ORDER BY name ASC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        Ok(qb.build_query_as::<KnownTestPlan>().fetch_all(conn).await?)
    }

    pub async fn n_matching(&self, conn: &mut PgConnection) -> Result<i64> {
        let mut qb = QueryBuilder::new("SELECT COUNT(*) FROM known_test_plan");
        self.push_where_clause(&mut qb);

        Ok(qb.build_query_scalar().fetch_one(conn).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::DEFAULT_POOL, conn, db::TestRun};
    use simple_test_case::test_case;
    use std::assert_matches;
    use uuid::Uuid;

    const OWNER: &str = "test-owner@apollographql.com";
    const TRIGGER: &str = "test-trigger@apollographql.com";

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn plan_with_stored_patch(workload_config_patch: Option<Value>) -> KnownTestPlan {
        KnownTestPlan {
            id: 1,
            uuid: Uuid::new_v4(),
            name: "plan".into(),
            description: None,
            org: "org".into(),
            repo: "repo".into(),
            path: "path".into(),
            pinned_workload_pool: None,
            workload_config_patch,
            requires_dedicated_cluster: false,
            restrict_triggers: false,
            restrict_automated_triggers: false,
        }
    }

    #[test]
    fn a_plan_with_no_stored_patch_has_an_empty_one() {
        let plan = plan_with_stored_patch(None);

        assert_eq!(
            plan.workload_config_patch().unwrap(),
            WorkloadConfigPatch::default()
        );
    }

    #[test]
    fn a_stored_patch_is_parsed() {
        let plan = plan_with_stored_patch(Some(serde_json::json!({
            "allow_k8s_write": true,
            "node_label_weights": [["a", 1]],
        })));

        assert_eq!(
            plan.workload_config_patch().unwrap(),
            WorkloadConfigPatch {
                allow_k8s_write: Some(true),
                node_label_weights: Some(vec![("a".into(), 1)]),
            }
        );
    }

    #[tokio::test]
    async fn a_malformed_stored_patch_doesnt_error_summary_creation() -> anyhow::Result<()> {
        let plan = plan_with_stored_patch(Some(serde_json::json!({
            "node_label_weights": "not a list",
        })));

        assert_matches!(
            plan.workload_config_patch(),
            Err(db::Error::MalformedWorkloadConfigPatch(_))
        );
        assert_eq!(
            plan.try_into_summary(conn!())
                .await
                .unwrap()
                .workload_config_patch,
            WorkloadConfigPatch::default()
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn insert_and_get_by_uuid_round_trip() -> Result<()> {
        let c = conn!();
        let name = unique("plan");
        let inserted =
            KnownTestPlan::register(&name, Some("desc"), "org", "repo", &unique("path"), c).await?;

        let fetched = KnownTestPlan::get_by_uuid(&inserted.uuid, c).await?;
        assert_eq!(fetched, Some(inserted));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_name_returns_matching_plan() -> Result<()> {
        let c = conn!();
        let name = unique("plan-by-name");
        let inserted =
            KnownTestPlan::register(&name, None, "org", "repo", &unique("path"), c).await?;

        let fetched = KnownTestPlan::get_by_name(&name, c).await?;
        assert_eq!(fetched, Some(inserted));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_uuid_returns_none_for_unknown_uuid() -> Result<()> {
        let c = conn!();
        let fetched = KnownTestPlan::get_by_uuid(&Uuid::new_v4(), c).await?;
        assert_eq!(fetched, None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn insert_rejects_duplicate_name() -> Result<()> {
        let c = conn!();
        let name = unique("dup-name");
        KnownTestPlan::register(&name, None, "org-a", "repo-a", &unique("path-a"), c).await?;

        let res =
            KnownTestPlan::register(&name, None, "org-b", "repo-b", &unique("path-b"), c).await;
        assert_matches!(res, Err(db::Error::KnownTestPlanAlreadyExists), "{res:?}");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn insert_rejects_duplicate_org_repo_path() -> Result<()> {
        let c = conn!();
        let org = unique("org");
        let repo = unique("repo");
        let path = unique("path");
        KnownTestPlan::register(&unique("name-a"), None, &org, &repo, &path, c).await?;

        let res = KnownTestPlan::register(&unique("name-b"), None, &org, &repo, &path, c).await;
        assert!(
            matches!(res, Err(db::Error::KnownTestPlanAlreadyExists)),
            "{res:?}"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn link_persists_the_git_sha() -> Result<()> {
        let c = conn!();
        let known =
            KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), c)
                .await?;
        let tr = TestRun::init_unknown_initiator(&unique("run"), None, &DEFAULT_POOL, c).await?;

        let link = KnownTestPlanRun::link(known.id(), tr.id(), Some("abc123"), c).await?;

        assert_eq!(link.known_test_plan_id, known.id());
        assert_eq!(link.test_run_id, tr.id());
        assert_eq!(link.git_sha.as_deref(), Some("abc123"));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn link_allows_a_null_git_sha() -> Result<()> {
        let c = conn!();
        let known =
            KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), c)
                .await?;
        let tr = TestRun::init_unknown_initiator(&unique("run"), None, &DEFAULT_POOL, c).await?;

        let link = KnownTestPlanRun::link(known.id(), tr.id(), None, c).await?;

        assert_eq!(link.git_sha, None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn matching_filters_by_name() -> Result<()> {
        let c = conn!();
        let name = unique("match-me");
        KnownTestPlan::register(&name, None, "org", "repo", &unique("path"), c).await?;
        KnownTestPlan::register(
            &unique("not-this-one"),
            None,
            "org",
            "repo",
            &unique("path"),
            c,
        )
        .await?;

        let filter = KnownTestPlanFilter {
            name: Some(name.clone()),
        };
        let results = filter.matching(10, 0, c).await?;

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].name(), name);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn matching_respects_pagination() -> Result<()> {
        let c = conn!();
        let org = unique("org");
        for name in ["a", "b", "c"] {
            KnownTestPlan::register(&unique(name), None, &org, "repo", &unique("path"), c).await?;
        }

        let filter = KnownTestPlanFilter { name: None };
        let n = filter.n_matching(c).await?;
        let page = filter.matching(1, 0, c).await?;

        assert!(n >= 3, "{n}");
        assert_eq!(page.len(), 1, "{page:?}");

        Ok(())
    }

    fn update_users_req(user_changes: &[(&str, Option<SharedRole>)]) -> UpdateKnownTestPlanRequest {
        UpdateKnownTestPlanRequest {
            associated_users: Some(
                user_changes
                    .iter()
                    .map(|(email, role)| (email.to_string(), *role))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test_case(
        &[],
        &[(OWNER, Role::Owner), (TRIGGER, Role::CanTrigger)];
        "no changes"
    )]
    #[test_case(
        &[(TRIGGER, Some(SharedRole::Owner))],
        &[(OWNER, Role::Owner), (TRIGGER, Role::Owner)];
        "upgrade trigger user to owner"
    )]
    #[test_case(
        &[(OWNER, Some(SharedRole::CanTrigger))],
        &[(OWNER, Role::CanTrigger), (TRIGGER, Role::CanTrigger)];
        "downgrade owner user to trigger"
    )]
    #[test_case(
        &[(TRIGGER, None)],
        &[(OWNER, Role::Owner)];
        "remove user"
    )]
    #[test_case(
        &[("alice@apollographql.com", Some(SharedRole::CanTrigger))],
        &[(OWNER, Role::Owner), (TRIGGER, Role::CanTrigger), ("alice@apollographql.com", Role::CanTrigger)];
        "add new user"
    )]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn user_updates_produce_correct_state(
        user_changes: &[(&str, Option<SharedRole>)],
        expected: &[(&str, Role)],
    ) -> Result<()> {
        let c = conn!();
        let plan =
            KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), c)
                .await?;

        // Set initial users
        let plan = plan
            .update(
                update_users_req(&[
                    (OWNER, Some(SharedRole::Owner)),
                    (TRIGGER, Some(SharedRole::CanTrigger)),
                ]),
                c,
            )
            .await?;

        // apply changes
        let plan = plan.update(update_users_req(user_changes), c).await?;

        assert_eq!(
            plan.associated_users(c).await?,
            BTreeMap::from_iter(
                expected
                    .iter()
                    .map(|(email, role)| (email.to_string(), *role))
            )
        );

        Ok(())
    }

    async fn setup_update_and_validate_test(
        user: KnownUser,
        req: UpdateKnownTestPlanRequest,
    ) -> crate::Result<KnownTestPlan> {
        let c = conn!();
        let plan =
            KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), c)
                .await
                .expect("failed to create known test plan")
                .update(
                    update_users_req(&[
                        (OWNER, Some(SharedRole::Owner)),
                        (TRIGGER, Some(SharedRole::CanTrigger)),
                    ]),
                    c,
                )
                .await
                .expect("failed to set initial owners");

        plan.validate_and_update(
            &user,
            req,
            &HashSet::from([DEFAULT_POOL_NAME]),
            &HashSet::from([DEFAULT_POOL_NAME]),
            c,
        )
        .await
    }

    fn admin() -> KnownUser {
        KnownUser::Admin("admin@apollographql.com".into())
    }

    fn user(email: &str) -> KnownUser {
        KnownUser::User(email.into())
    }

    // Update requests for the permissions check tests below

    fn pin_workload_pool() -> UpdateKnownTestPlanRequest {
        UpdateKnownTestPlanRequest {
            pinned_workload_pool: Some(Some(DEFAULT_POOL_NAME.into())),
            ..Default::default()
        }
    }

    fn requires_dedicated_cluster() -> UpdateKnownTestPlanRequest {
        UpdateKnownTestPlanRequest {
            requires_dedicated_cluster: Some(false),
            ..Default::default()
        }
    }

    fn change_description() -> UpdateKnownTestPlanRequest {
        UpdateKnownTestPlanRequest {
            description: Some(Some("new description".into())),
            ..Default::default()
        }
    }

    fn add_trigger_user() -> UpdateKnownTestPlanRequest {
        update_users_req(&[("alice@apollographql.com", Some(SharedRole::CanTrigger))])
    }

    fn add_owner() -> UpdateKnownTestPlanRequest {
        update_users_req(&[("alice@apollographql.com", Some(SharedRole::Owner))])
    }

    fn remove_trigger_user() -> UpdateKnownTestPlanRequest {
        update_users_req(&[(TRIGGER, None)])
    }

    fn remove_owner() -> UpdateKnownTestPlanRequest {
        update_users_req(&[(OWNER, None)])
    }

    fn demote_owner() -> UpdateKnownTestPlanRequest {
        update_users_req(&[(OWNER, Some(SharedRole::CanTrigger))])
    }

    fn promote_trigger_user() -> UpdateKnownTestPlanRequest {
        update_users_req(&[(TRIGGER, Some(SharedRole::Owner))])
    }

    // admins should be able to perform every action
    #[test_case(admin(), pin_workload_pool(); "admin can pin workload pool")]
    #[test_case(admin(), requires_dedicated_cluster(); "admin can require dedicated cluster")]
    #[test_case(admin(), change_description(); "admin can change non-admin field")]
    #[test_case(admin(), add_trigger_user(); "admin can add a trigger user")]
    #[test_case(admin(), add_owner(); "admin can add an owner")]
    #[test_case(admin(), remove_trigger_user(); "admin can remove a trigger user")]
    #[test_case(admin(), remove_owner(); "admin can remove an owner")]
    #[test_case(admin(), demote_owner(); "admin can demote an owner to a trigger user")]
    #[test_case(admin(), promote_trigger_user(); "admin can promote a trigger user to owner")]
    // owners should be able to perform the following subset of actions
    #[test_case(user(OWNER), change_description(); "owner can change non-admin field")]
    #[test_case(user(OWNER), add_trigger_user(); "owners can add a new user")]
    #[test_case(user(OWNER), remove_trigger_user(); "owners can remove a trigger user")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn validate_and_update_happy_path(user: KnownUser, req: UpdateKnownTestPlanRequest) {
        let res = setup_update_and_validate_test(user, req).await;

        assert!(res.is_ok(), "{res:?}");
    }

    #[test_case(pin_workload_pool(); "pin to a workload pool")]
    #[test_case(requires_dedicated_cluster(); "require a dedicated cluster")]
    #[test_case(add_owner(); "add new owner")]
    #[test_case(remove_owner(); "remove existing owner")]
    #[test_case(demote_owner(); "demote owner to a trigger user")]
    #[test_case(promote_trigger_user(); "promote a trigger user to owner")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn validate_and_update_admin_only(req: UpdateKnownTestPlanRequest) {
        let res = setup_update_and_validate_test(user(OWNER), req).await;

        assert_matches!(res, Err(crate::Error::AdminOnlyField { .. }), "{res:?}");
    }

    #[test_case(user(TRIGGER), change_description(); "trigger user")]
    #[test_case(user("alice@apollographql.com"), change_description(); "unassociated user")]
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn validate_and_update_unauthorized(user: KnownUser, req: UpdateKnownTestPlanRequest) {
        let res = setup_update_and_validate_test(user, req).await;

        assert_matches!(res, Err(crate::Error::Unauthorized), "{res:?}");
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn try_into_summary_includes_associated_users() -> Result<()> {
        let c = conn!();
        let (owner, trigger) = (unique("owner"), unique("trigger"));
        let plan =
            KnownTestPlan::register(&unique("plan"), None, "org", "repo", &unique("path"), c)
                .await?;
        let plan = plan
            .update(
                UpdateKnownTestPlanRequest {
                    restrict_triggers: Some(true),
                    restrict_automated_triggers: Some(false),
                    associated_users: update_users_req(&[
                        (&owner, Some(SharedRole::Owner)),
                        (&trigger, Some(SharedRole::CanTrigger)),
                    ])
                    .associated_users,
                    ..Default::default()
                },
                c,
            )
            .await?;

        let summary = plan.try_into_summary(c).await?;

        assert!(summary.restrict_triggers);
        assert!(!summary.restrict_automated_triggers);
        assert_eq!(
            summary.associated_users,
            BTreeMap::from([
                (owner, SharedRole::Owner),
                (trigger, SharedRole::CanTrigger)
            ])
        );

        Ok(())
    }
}
