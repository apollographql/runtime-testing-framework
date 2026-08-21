use crate::db::{
    self, ClusterId, Queryable, Result,
    status::{Status, StatusTracked, StatusUpdate},
    test_execution::TestExecution,
};
use chrono::{DateTime, Utc};
use rtf_orchestrator_shared::{payload::PreparedPayload, summary::TestRunSummary};
use serde_json::Value;
use sqlx::{FromRow, PgConnection};
use std::collections::HashMap;
use tracing::error;
use uuid::Uuid;

/// Reported in place of a `None` `initiated_by` when a [`TestRun`] is turned into a
/// [`TestRunSummary`]. Also used by [`super::TestRunFilter`] so that searching for this value
/// finds runs with no recorded initiator, matching what the UI actually displays for them.
pub(crate) const UNKNOWN_INITIATOR: &str = "unknown";

/// A `TestRun` denotes a single user-triggered set of tests that should be considered passing or
/// failing based on their combined status.
///
/// This is a logical construct that we use to make it easier to trigger and query the results of
/// workloads submitted to the orchestrator. Each [TestRun] contains one or more [TestExecution]s
/// which denote the actual tests being run.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TestRun {
    id: i32,
    uuid: Uuid,
    name: String,
    initiated_by: Option<String>,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    variables_id: Option<i32>,
    workload_cluster: String,
}

impl Queryable for TestRun {
    const TABLE_NAME: &'static str = "test_run";

    fn id(&self) -> i32 {
        self.id
    }
}

impl StatusTracked for TestRun {
    const STATUS_TABLE: &'static str = "test_run_status";

    // No additional logic needed when recording status items
    async fn after_set_status(&self, _status: Status, _conn: &mut PgConnection) -> Result<()> {
        Ok(())
    }
}

impl TestRun {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn initiated_by(&self) -> Option<&str> {
        self.initiated_by.as_deref()
    }

    pub fn workload_cluster(&self) -> ClusterId {
        ClusterId::new(&self.workload_cluster)
    }

    #[cfg(test)]
    pub fn create_stub(id: i32, name: &str) -> Self {
        Self {
            id,
            uuid: Uuid::new_v4(),
            name: name.into(),
            initiated_by: None,
            started_at: Utc::now(),
            completed_at: None,
            variables_id: None,
            workload_cluster: "alpha".into(),
        }
    }

    pub async fn get_by_uuid(uuid: &Uuid, conn: &mut PgConnection) -> Result<Option<Self>> {
        Ok(sqlx::query_as("SELECT * FROM test_run WHERE uuid = $1;")
            .bind(uuid)
            .fetch_optional(conn)
            .await?)
    }

    /// Create a new run, capturing any runtime variable overrides supplied with it and the test run
    /// initiator.
    ///
    /// `variables` is the flat overrides blob (`-v` / `--vars`) as JSON, or `None` when none were
    /// supplied. When present it is upserted into the deduplicated `variables` table and the run is
    /// linked to it, so `variables_id` is `NULL` exactly when `variables` is `None`.
    ///
    /// `initiated_by` is whatever identity the caller managed to extract, or `None` if it couldn't
    /// (or didn't try to). Stored as-is — `None` is persisted as SQL `NULL`, not some sentinel
    /// string, so "we don't know" stays a real, queryable absence rather than a magic value.
    ///
    /// `workload_cluster` is the identifier of the workload cluster this run's executions will run
    /// in, resolved by the caller (e.g. a known test plan's pin, or the server's configured
    /// default cluster).
    pub async fn init(
        name: &str,
        variables: Option<Value>,
        initiated_by: Option<&str>,
        workload_cluster: &ClusterId,
        conn: &mut PgConnection,
    ) -> Result<Self> {
        let variables_id = match variables {
            Some(data) => Some(db::upsert_variables(&data, conn).await?),
            None => None,
        };

        let tr: TestRun = sqlx::query_as(
            "INSERT INTO test_run (name, started_at, variables_id, initiated_by, workload_cluster)
             VALUES ($1, NOW(), $2, $3, $4)
             RETURNING id, uuid, name, initiated_by, started_at, completed_at, variables_id, workload_cluster;
            ",
        )
        .bind(name)
        .bind(variables_id)
        .bind(initiated_by)
        .bind(workload_cluster.as_str())
        .fetch_one(&mut *conn)
        .await?;

        tr.set_status(Status::Initialising, None, conn).await?;

        Ok(tr)
    }

    /// Initialise a TestRun, setting `initiated_by`` to `None`
    pub async fn init_unknown_initiator(
        name: &str,
        variables: Option<Value>,
        workload_cluster: &ClusterId,
        conn: &mut PgConnection,
    ) -> Result<Self> {
        Self::init(name, variables, None, workload_cluster, conn).await
    }

    pub async fn init_execution(
        &self,
        name: &str,
        index: usize,
        conn: &mut PgConnection,
    ) -> Result<TestExecution> {
        TestExecution::init(name, self.id, index, conn).await
    }

    pub async fn executions(&self, conn: &mut PgConnection) -> Result<Vec<TestExecution>> {
        Ok(
            sqlx::query_as("SELECT * FROM test_execution WHERE test_run_id = $1 ORDER BY id;")
                .bind(self.id)
                .fetch_all(conn)
                .await?,
        )
    }

    /// Following a status update for a child [TestExecution] we need to determine whether or not
    /// the overall status of this [TestRun] needs to be updated. This method will return
    /// `Some(status)` if the given `execution_status` triggers an update for the run as a whole,
    /// otherwise `None`.
    pub(super) async fn status_after_execution_update(
        &self,
        execution_status: Status,
        conn: &mut PgConnection,
    ) -> Result<Option<Status>> {
        let StatusUpdate {
            status: run_status, ..
        } = self.current_status(conn).await?;

        status_after_execution_update(run_status, execution_status, async move || {
            let sibling_executions = self.executions(conn).await?;
            let mut execution_statuses = Vec::with_capacity(sibling_executions.len());
            for ex in sibling_executions.iter() {
                execution_statuses.push(ex.current_status(conn).await?.status);
            }

            Ok(execution_statuses)
        })
        .await
    }

    pub async fn try_into_summary(self, conn: &mut PgConnection) -> Result<TestRunSummary> {
        let trigger_variables = self.variables(conn).await?;
        let status_history = self.status_history(conn).await?;
        let current = status_history
            .first()
            .cloned()
            .ok_or(db::Error::Sqlx(sqlx::Error::RowNotFound))?;

        Ok(TestRunSummary {
            id: self.uuid,
            name: self.name,
            trigger_variables,
            current_status: current.status.into(),
            initiated_by: self
                .initiated_by
                .unwrap_or_else(|| UNKNOWN_INITIATOR.to_owned()),
            started_at: self.started_at,
            updated_at: current.updated_at,
            completed_at: self.completed_at,
            status_history: status_history.into_iter().map(Into::into).collect(),
            executions: Vec::new(),
        })
    }

    pub async fn try_into_summary_with_executions(
        self,
        conn: &mut PgConnection,
    ) -> Result<TestRunSummary> {
        let raw_executions = self.executions(conn).await?;
        let mut summary = self.try_into_summary(conn).await?;

        summary.executions = Vec::with_capacity(raw_executions.len());
        for ex in raw_executions.into_iter() {
            // It is possible for us to encounter TestExecutions that are part way through
            // initialising when calling this method (as in, the row for the execution exists
            // in the DB but we don't yet have the first status row). Building a summary requires
            // at least one status, so we skip executions missing that information.
            //
            // The user facing effect of this is the same as if the main execution row had
            // not yet been created, namely that the execution is not yet present in the list
            match ex.try_into_summary(conn).await {
                Ok(ex_summary) => summary.executions.push(ex_summary),
                Err(db::Error::Sqlx(sqlx::Error::RowNotFound)) => continue,
                Err(e) => return Err(e),
            }
        }

        Ok(summary)
    }

    /// Load all currently cached test plans into a map of test run UUID to ([TestRun], [PreparedPayload]).
    ///
    /// Returns DB level errors as `Err` but partitions off malformed test plan JSON errors into a
    /// [Vec] of [TestRun]s that it is the caller's responsibility to process. To evict malformed
    /// data from the cache use [TestRun::clear_cached_payload]. To evict the entire cache use
    /// [TestRun::clear_payload_cache].
    pub async fn load_payload_cache(
        conn: &mut PgConnection,
    ) -> Result<(HashMap<Uuid, (TestRun, PreparedPayload)>, Vec<TestRun>)> {
        let raw = CachedPayload::load_all(conn).await?;
        let mut map = HashMap::with_capacity(raw.len());
        let mut malformed = Vec::new();

        for CachedPayload { run_id, payload } in raw.into_iter() {
            let tr = TestRun::get_by_id_unchecked(run_id, conn).await?;
            match serde_json::from_value(payload) {
                Ok(tp) => {
                    map.insert(tr.uuid, (tr, tp));
                }

                Err(err) => {
                    error!(%err, run_id=%tr.uuid, "malformed cached test plan");
                    malformed.push(tr);
                }
            }
        }

        Ok((map, malformed))
    }

    /// Clear the entire test plan cache.
    ///
    /// This does _not_ alter the status of associated runs and executions.
    pub async fn clear_payload_cache(conn: &mut PgConnection) -> Result<()> {
        sqlx::query("DELETE FROM payload_cache;")
            .execute(conn)
            .await?;

        Ok(())
    }

    /// Cache the given [PreparedPayload] against this [TestRun]s ID.
    ///
    /// Used to recover event loop state on startup for ongoing executions.
    pub async fn cache_payload(
        &self,
        payload: &PreparedPayload,
        conn: &mut PgConnection,
    ) -> Result<()> {
        let val = serde_json::to_value(payload).expect("payload to serialize");

        sqlx::query(
            "INSERT INTO payload_cache (run_id, payload)
             VALUES ($1, $2);
            ",
        )
        .bind(self.id)
        .bind(val)
        .execute(conn)
        .await?;

        Ok(())
    }

    /// Evict the cached test plan associated with a given [TestRun] UUID.
    pub async fn clear_cached_payload(run_uuid: Uuid, conn: &mut PgConnection) -> Result<()> {
        sqlx::query(
            "DELETE FROM payload_cache
             WHERE run_id IN (SELECT id FROM test_run WHERE uuid = $1);",
        )
        .bind(run_uuid)
        .execute(conn)
        .await?;

        Ok(())
    }

    async fn variables(&self, conn: &mut PgConnection) -> Result<Option<Value>> {
        let id = match self.variables_id {
            Some(id) => id,
            None => return Ok(None),
        };

        Ok(
            sqlx::query_scalar("SELECT data from variables WHERE id = $1;")
                .bind(id)
                .fetch_one(conn)
                .await?,
        )
    }
}

async fn status_after_execution_update<F, Fut>(
    run_status: Status,
    execution_status: Status,
    get_sibling_statuses: F,
) -> Result<Option<Status>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<Status>>>,
{
    use Status::*;

    let new_run_status = match (run_status, execution_status) {
        // Once at least one execution reports resolving, the run as a whole is resolving
        (Initialising, Resolving) => Some(Resolving),

        // Once at least one execution reports provisioning, the run as a whole is provisioning
        (Initialising | Resolving, Provisioning) => Some(Provisioning),

        // Once at least one execution reports environment_ready, the run as a whole is
        // environment_ready (Argo workflow completed, scenario job being set up)
        (Initialising | Resolving | Provisioning, EnvironmentReady) => Some(EnvironmentReady),

        // Once at least one execution reports running, the run as a whole is running
        (Initialising | Resolving | Provisioning | EnvironmentReady, Running) => Some(Running),

        // Once all executions are complete we can determine the terminal status of the run.
        // Once the run has a terminal status, further updates are ignored
        (s_run, s_ex) if s_ex.is_terminal() && !s_run.is_terminal() => {
            let execution_statuses = (get_sibling_statuses)().await?;

            // Combine the statuses of all executions in this run. If the result is a terminal
            // status then we need to update.
            execution_statuses
                .into_iter()
                .reduce(|l, r| l.combine(r))
                .filter(|&s| s.is_terminal())
        }

        _ => None,
    };

    Ok(new_run_status)
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct CachedPayload {
    run_id: i32,
    payload: Value,
}

impl Queryable for CachedPayload {
    const TABLE_NAME: &'static str = "payload_cache";

    fn id(&self) -> i32 {
        self.run_id
    }
}

impl CachedPayload {
    async fn load_all(conn: &mut PgConnection) -> Result<Vec<Self>> {
        Ok(sqlx::query_as("SELECT run_id, payload FROM payload_cache;")
            .fetch_all(conn)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conn,
        db::status::{Status, StatusTracked},
    };
    use Status::*;
    use rtf_config::{
        formats::{
            DockerCommand, DockerComposeEnvironment, DockerScenario, EnvironmentConfig,
            OutputCollection, ScenarioConfig,
        },
        templating::Field,
    };
    use rtf_orchestrator_shared::{
        payload::SourceKeyedArrayMap,
        test_plan::{OrchestratorEnvironment, OrchestratorTestPlan},
    };
    use serde_json::json;
    use simple_test_case::test_case;

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    fn perf_cluster() -> ClusterId {
        ClusterId::new("router_perf")
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_creates_run_with_initialising_status() -> Result<()> {
        let c = conn!();
        let res = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await;
        assert!(res.is_ok(), "{res:?}");

        let tr = res.unwrap();
        assert_eq!(tr.name, "test", "{tr:?}");

        let current = tr.current_status(c).await?;
        assert_eq!(current.status, Status::Initialising);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_leaves_variables_id_null() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        assert_eq!(tr.variables_id, None);

        // Persisted as NULL too, not just on the returned struct.
        let fetched = TestRun::get_by_id_unchecked(tr.id, c).await?;
        assert_eq!(fetched.variables_id, None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_leaves_initiated_by_none_by_default() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        assert_eq!(tr.initiated_by, None);

        // Persisted as a real NULL, not just on the returned struct.
        let fetched = TestRun::get_by_id_unchecked(tr.id, c).await?;
        assert_eq!(fetched.initiated_by, None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_persists_workload_cluster() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &perf_cluster(), c).await?;
        assert_eq!(tr.workload_cluster(), perf_cluster());

        // Persisted, not just present on the returned struct.
        let fetched = TestRun::get_by_id_unchecked(tr.id, c).await?;
        assert_eq!(fetched.workload_cluster(), perf_cluster());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn try_into_summary_reports_a_none_initiator_as_unknown() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;

        let summary = tr.try_into_summary(c).await?;
        assert_eq!(summary.initiated_by, "unknown");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_persists_and_dedupes_variables() -> Result<()> {
        let c = conn!();
        let vars = json!({ "region": "us", "tier": [1, 2, 3] });

        let tr = TestRun::init_unknown_initiator("test", Some(vars.clone()), &alpha_cluster(), c)
            .await?;
        let captured = tr.variables_id;
        assert!(
            captured.is_some(),
            "a run with overrides should have a variables_id"
        );

        // Persisted, not just present on the returned struct.
        let fetched = TestRun::get_by_id_unchecked(tr.id, c).await?;
        assert_eq!(fetched.variables_id, captured);

        // Dedup: upserting the same blob directly yields the same id init stored.
        let direct = db::upsert_variables(&vars, c).await?;
        assert_eq!(
            captured,
            Some(direct),
            "identical overrides should dedupe to a single variables row"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_id_returns_matching_run() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        let tr2 = TestRun::get_by_id(tr1.id, c).await?;

        assert_eq!(Some(tr1), tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_id_unchecked_returns_matching_run() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        let tr2 = TestRun::get_by_id_unchecked(tr1.id, c).await?;

        assert_eq!(tr1, tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn get_by_uuid_returns_matching_run() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        let tr2 = TestRun::get_by_uuid(&tr1.uuid, c).await?;

        assert_eq!(Some(tr1), tr2);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn executions_returns_all_associated_executions() -> Result<()> {
        let c = conn!();

        let tr = TestRun::init_unknown_initiator("A", None, &alpha_cluster(), c).await?;
        let ex1 = tr.init_execution("a", 0, c).await?;
        let ex2 = tr.init_execution("b", 1, c).await?;

        let executions = tr.executions(c).await?;
        assert_eq!(executions.len(), 2, "wrong number of executions");
        assert_eq!(executions[0], ex1, "execution 1");
        assert_eq!(executions[1], ex2, "execution 2");

        Ok(())
    }

    // Status of Initialising is checked in `init_creates_run_with_initialising_status` above
    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Resolving; "resolving")]
    #[test_case(Status::Provisioning; "provisioning")]
    #[test_case(Status::Running; "running")]
    #[test_case(Status::Successful; "successful")]
    #[test_case(Status::Failed; "failed")]
    #[test_case(Status::Unrunnable; "unrunnable")]
    #[tokio::test]
    async fn set_status_and_current_status_match(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;

        tr.set_status(status, None, c).await?;
        let current = tr.current_status(c).await?;

        assert_eq!(current.status, status);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn status_history_returns_entries_newest_first() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?; // sets Status::Initialising
        tr.set_status(Status::Running, None, c).await?;
        tr.set_status(Status::Successful, None, c).await?;

        let history = tr.status_history(c).await?;
        assert_eq!(history.len(), 3, "wrong number of history entries");
        assert_eq!(history[0].status, Status::Successful, "newest");
        assert_eq!(history[1].status, Status::Running, "second");
        assert_eq!(history[2].status, Status::Initialising, "oldest");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Successful; "successful")]
    #[test_case(Status::Failed; "failed")]
    #[test_case(Status::Unrunnable; "unrunnable")]
    #[tokio::test]
    async fn set_terminal_status_sets_completed_at(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        assert!(tr.completed_at.is_none());

        tr.set_status(status, None, c).await?;
        let tr = TestRun::get_by_id_unchecked(tr.id(), c).await?;
        assert!(tr.completed_at.is_some());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[test_case(Status::Initialising; "initialising")]
    #[test_case(Status::Resolving; "resolving")]
    #[test_case(Status::Provisioning; "provisioning")]
    #[test_case(Status::Running; "running")]
    #[tokio::test]
    async fn set_non_terminal_status_does_not_set_completed_at(status: Status) -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        assert!(tr.completed_at.is_none());

        tr.set_status(status, None, c).await?;
        let tr = TestRun::get_by_id_unchecked(tr.id(), c).await?;
        assert!(tr.completed_at.is_none());

        Ok(())
    }

    // First execution to hit Resolving/Provisioning/Running should update
    #[test_case(Initialising, Resolving, &[], Some(Resolving); "init to resolving")]
    #[test_case(Initialising, Provisioning, &[], Some(Provisioning); "init to provisioning")]
    #[test_case(Initialising, Running, &[], Some(Running); "init to running")]
    #[test_case(Resolving, Provisioning, &[], Some(Provisioning); "resolving to provisioning")]
    #[test_case(Resolving, Running, &[], Some(Running); "resolving to running")]
    #[test_case(Provisioning, Running, &[], Some(Running); "provisioning to running")]
    // Moving to Resolving/Provisioning/Running should only happen once
    #[test_case(Resolving, Resolving, &[], None; "already resolving")]
    #[test_case(Provisioning, Provisioning, &[], None; "already provisioning")]
    #[test_case(Running, Running, &[], None; "already running")]
    // Successful while siblings are ongoing
    #[test_case(Running, Successful, &[Running], None; "successful sibling running")]
    // Non-successful terminal while siblings are ongoing
    #[test_case(Running, Failed, &[Running, Successful], Some(Failed); "failed sibling running")]
    #[test_case(Running, Unrunnable, &[Running, Successful], Some(Unrunnable); "unrunnable sibling running")]
    // Last execution reporting terminal status
    #[test_case(Running, Successful, &[Successful], Some(Successful); "final successful")]
    #[test_case(Running, Unrunnable, &[Successful], Some(Unrunnable); "final unrunnable")]
    #[test_case(Running, Failed, &[Successful], Some(Failed); "final failed")]
    // Failed and Unrunnable eagerly update run status, so further terminal execution updates
    // should be ignored
    #[test_case(Failed, Successful, &[Failed], None; "final successful but already failed")]
    #[test_case(Unrunnable, Successful, &[Unrunnable], None; "final successful but already unrunnable")]
    #[tokio::test]
    async fn status_after_execution_update(
        run_status: Status,
        ex_status: Status,
        siblings: &[Status],
        expected: Option<Status>,
    ) -> Result<()> {
        let mut ex_statuses = siblings.to_vec();
        ex_statuses.push(ex_status);

        let new_status =
            status_after_execution_update(run_status, ex_status, async move || Ok(ex_statuses))
                .await?;

        assert_eq!(new_status, expected);

        Ok(())
    }

    fn stub_payload() -> PreparedPayload {
        PreparedPayload {
            variables: None,
            test_plan: OrchestratorTestPlan {
                name: String::new(),
                description: String::new(),
                variables: Default::default(),
                matrix: Default::default(),
                custom_providers: vec![],
                scenario: ScenarioConfig {
                    name: String::new(),
                    description: String::new(),
                    variable_definitions: vec![],
                    custom_providers: vec![],
                    execution: DockerScenario {
                        docker: DockerCommand {
                            image: Field::Resolved("nginx".into()),
                            tag: None,
                            command: Field::Resolved("echo test".into()),
                        },
                        env_vars: Default::default(),
                        file_providers: vec![],
                        output_collection: OutputCollection { prometheus: vec![] },
                    },
                },
                environment: EnvironmentConfig {
                    name: String::new(),
                    description: String::new(),
                    variable_definitions: vec![],
                    custom_providers: vec![],
                    execution: OrchestratorEnvironment::DockerCompose(DockerComposeEnvironment {
                        project_name: None,
                        compose_files: vec![],
                        file_providers: vec![],
                        env_vars: Default::default(),
                        output_collection: OutputCollection { prometheus: vec![] },
                    }),
                },
            },
            relative_files: SourceKeyedArrayMap::from_data(HashMap::new()),
            custom_providers: SourceKeyedArrayMap::from_data(HashMap::new()),
        }
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn load_test_plan_cache_returns_cached_plans() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init_unknown_initiator("run-a", None, &alpha_cluster(), c).await?;
        let tr2 = TestRun::init_unknown_initiator("run-b", None, &alpha_cluster(), c).await?;
        let uuid1 = tr1.uuid();
        let uuid2 = tr2.uuid();

        tr1.cache_payload(&stub_payload(), c).await?;
        tr2.cache_payload(&stub_payload(), c).await?;

        let (map, _) = TestRun::load_payload_cache(c).await?;
        assert!(map.contains_key(&uuid1), "run-a UUID not in cache");
        assert!(map.contains_key(&uuid2), "run-b UUID not in cache");
        assert!(map[&uuid1].0.uuid() == uuid1, "run-a TestRun in map");
        assert!(map[&uuid2].0.uuid() == uuid2, "run-b TestRun in map");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn load_test_plan_cache_partitions_malformed_json() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        let uuid = tr.uuid();

        // We don't expose an API for storing an arbitrary JSON blob like this, but we need to
        // guard against structural changes in OrchestratorTestPlan meaning that we somehow manage to have
        // old data that no longer parses present in the cache.
        sqlx::query("INSERT INTO payload_cache (run_id, payload) VALUES ($1, $2::jsonb)")
            .bind(tr.id())
            .bind(json!({"not": "a trigger payload"}))
            .execute(&mut *c)
            .await?;

        let (map, malformed) = TestRun::load_payload_cache(c).await?;
        assert!(!map.contains_key(&uuid), "present in map");
        assert!(
            malformed.iter().any(|r| r.uuid() == uuid),
            "present in malformed"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn clear_cached_payload_removes_the_entry() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        let run_id = tr.id;

        // should be present in the cache after caching
        tr.cache_payload(&stub_payload(), c).await?;

        let cache = CachedPayload::load_all(c).await?;
        assert!(
            cache.iter().any(|elem| elem.run_id == run_id),
            "should be cached"
        );

        // should be removed from the cache after clearing
        TestRun::clear_cached_payload(tr.uuid(), c).await?;

        let cache = CachedPayload::load_all(c).await?;
        assert!(
            cache.iter().all(|elem| elem.run_id != run_id),
            "should not be cached"
        );

        Ok(())
    }

    #[tokio::test]
    #[ignore = "races with other tests that use the test plan cache"]
    async fn clear_test_plan_cache_removes_all_entries() -> Result<()> {
        let c = conn!();
        let tr1 = TestRun::init_unknown_initiator("run-a", None, &alpha_cluster(), c).await?;
        let tr2 = TestRun::init_unknown_initiator("run-b", None, &alpha_cluster(), c).await?;
        let uuid1 = tr1.uuid();
        let uuid2 = tr2.uuid();

        tr1.cache_payload(&stub_payload(), c).await?;
        tr2.cache_payload(&stub_payload(), c).await?;
        let (map, _) = TestRun::load_payload_cache(c).await?;
        assert!(
            map.contains_key(&uuid1),
            "run-a should be present before clearing"
        );
        assert!(
            map.contains_key(&uuid2),
            "run-b should be present before clearing"
        );

        TestRun::clear_payload_cache(c).await?;

        let (map, malformed) = TestRun::load_payload_cache(c).await?;
        assert!(map.is_empty(), "expected empty map after clear all");
        assert!(malformed.is_empty(), "unexpected malformed after clear all");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn try_into_summary_leaves_executions_empty() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        tr.init_execution("exec", 0, c).await?;

        let uuid = tr.uuid();
        let summary = tr.try_into_summary(c).await?;

        assert_eq!(summary.id, uuid);
        assert!(summary.executions.is_empty(), "{summary:?}");
        assert_eq!(summary.current_status, Status::Initialising.into());

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn try_into_summary_returns_trigger_variables() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator(
            "test",
            Some(json!({"foo": "bar"})),
            &alpha_cluster(),
            c,
        )
        .await?;
        tr.init_execution("exec", 0, c).await?;

        let uuid = tr.uuid();
        let summary = tr.try_into_summary(c).await?;

        assert_eq!(summary.id, uuid);
        assert_eq!(summary.trigger_variables, Some(json!({"foo": "bar"})));

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn try_into_summary_with_executions_includes_executions() -> Result<()> {
        let c = conn!();
        let tr = TestRun::init_unknown_initiator("test", None, &alpha_cluster(), c).await?;
        tr.init_execution("exec", 0, c).await?;

        let summary = tr.try_into_summary_with_executions(c).await?;
        assert_eq!(summary.executions.len(), 1, "{summary:?}");

        Ok(())
    }
}
