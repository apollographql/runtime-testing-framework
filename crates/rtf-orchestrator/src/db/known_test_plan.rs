use crate::db::{self, ClusterId, Queryable, Result};
use rtf_orchestrator_shared::known_test_plan::KnownTestPlanSummary;
use sqlx::{PgConnection, Postgres, QueryBuilder};
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
    pinned_workload_cluster: Option<String>,
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

    pub fn pinned_workload_cluster(&self) -> Option<ClusterId> {
        self.pinned_workload_cluster.as_ref().map(ClusterId::new)
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
              id, uuid, name, description, org, repo, path, pinned_workload_cluster;
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

    pub async fn set_pinned_workload_cluster(
        &mut self,
        pinned_workload_cluster: Option<&str>,
        conn: &mut PgConnection,
    ) -> Result<()> {
        sqlx::query("UPDATE known_test_plan SET pinned_workload_cluster = $1 WHERE id = $2;")
            .bind(pinned_workload_cluster)
            .bind(self.id)
            .execute(conn)
            .await?;

        self.pinned_workload_cluster = pinned_workload_cluster.map(str::to_owned);

        Ok(())
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

    pub fn into_summary(self) -> KnownTestPlanSummary {
        KnownTestPlanSummary {
            uuid: self.uuid,
            name: self.name,
            description: self.description,
            org: self.org,
            repo: self.repo,
            path: self.path,
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
    fn push_where_clause(&self, qb: &mut QueryBuilder<'_, Postgres>) {
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
    use crate::{conn, db::TestRun};
    use std::assert_matches;
    use uuid::Uuid;

    fn unique(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
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
        let tr = TestRun::init_unknown_initiator(&unique("run"), None, &alpha_cluster(), c).await?;

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
        let tr = TestRun::init_unknown_initiator(&unique("run"), None, &alpha_cluster(), c).await?;

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
}
