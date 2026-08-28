use crate::{
    config::{Config, PerUserExecutionConfig},
    conn,
    context::OrchestratorContext,
    db::{ClusterId, KnownTestPlan, KnownTestPlanRun, Queryable, TestRun},
    error::{Error, RateLimitReason},
    event_loop::{EventQueueState, SubmitError},
    state::{ServerState, UserType},
};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{Duration, Utc};
use rtf_orchestrator_shared::{
    payload::{GitHubPayload, PreparedPayload, TriggerPayload},
    summary::TestRunSummary,
};
use serde_json::Value;
use sqlx::PgConnection;
use tracing::{debug, info};

pub async fn handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(trigger_payload): Json<TriggerPayload>,
) -> Result<Json<TestRunSummary>, Error> {
    let user = state.identify_user(&headers).await?;
    let default_cluster = state.eq_state.default_cluster();
    let conn = conn!();
    let cfg = Config::get();
    let payload = PayloadWithMeta::resolve(trigger_payload, default_cluster, conn).await?;
    let per_user_cfg = cfg.per_user_execution_config(&payload.cluster)?;

    let queued_executions = match &user {
        UserType::User(email) => {
            apply_rate_limits(
                &state.eq_state,
                &per_user_cfg,
                &payload.cluster,
                email,
                conn,
            )
            .await?
        }

        // Admin users don't get rate limited
        UserType::Admin(_) => 0,
        // Anonymous users are only possible via local triggers: also not rate limited
        UserType::Unknown => 0,
    };

    let (cluster, known_link, payload) = payload.finish(cfg).await?;

    // The max queued executions limit can only be enforced once we have the actual test plan and
    // can check the number of executions it will result in.
    if !user.is_admin()
        && queued_executions + payload.test_plan.matrix.n_variants()
            > per_user_cfg.max_queued_executions
    {
        return Err(Error::RateLimited {
            cluster,
            reason: RateLimitReason::QueuedExecutions {
                current: queued_executions as u64,
                max: per_user_cfg.max_queued_executions as u64,
            },
        });
    }

    let ctx = OrchestratorContext::new_from_inlined_files(
        cfg,
        payload.relative_files.clone(),
        payload.custom_providers.clone(),
    );

    debug!("validating test plan file provider usage");
    ctx.validate_environment_file_provider_usage(&payload.test_plan)
        .await?;

    debug!("reserving pending executions");
    let claim = state
        .eq_state
        .try_reserve_pending_executions(&payload.test_plan)
        .await
        .ok_or(Error::InsufficientCapacity)?;

    // Capture any runtime variable overrides before creating the run so the run is born with the
    // correct variables_id (NULL iff there were no overrides).
    let variables = payload
        .variables
        .as_ref()
        .map(|v| serde_json::to_value(v).expect("variables to serialize"));

    let initiated_by = user.into_user_email();

    debug!("initialising run");
    let (test_run, summary) =
        match init_run_and_build_summary(&payload.test_plan.name, variables, initiated_by, cluster)
            .await
        {
            Ok((tr, s)) => (tr, s),
            Err(e) => {
                state.eq_state.release_pending_execution_claim(claim).await;
                return Err(e);
            }
        };

    if let Some(link) = known_link {
        debug!("linking run to known test plan");
        let res = KnownTestPlanRun::link(
            link.known_test_plan_id,
            test_run.id(),
            link.git_sha.as_deref(),
            conn,
        )
        .await;

        if let Err(e) = res {
            state.eq_state.release_pending_execution_claim(claim).await;
            return Err(e.into());
        }
    }

    debug!("submitting test plan");
    match state
        .eq_state
        .try_submit_test_plan(claim, test_run, payload)
        .await
    {
        Ok(()) => Ok(Json(summary)),

        // Our claim gets released for us by try_submit_test_plan in this case so we are safe to
        // just return the error.
        Err(SubmitError::ResolveChannelClosed) => Err(Error::ResolverChannelClosed),

        Err(SubmitError::InvalidClaim) => {
            unreachable!("claim is made using the submitted test plan")
        }
    }
}

async fn apply_rate_limits(
    eq_state: &EventQueueState,
    per_user: &PerUserExecutionConfig,
    cluster: &ClusterId,
    email: &str,
    conn: &mut PgConnection,
) -> Result<usize, Error> {
    let counts = eq_state.user_queue_counts(email, cluster).await;

    if counts.ongoing_runs >= per_user.max_concurrent_runs {
        return Err(Error::RateLimited {
            cluster: cluster.clone(),
            reason: RateLimitReason::ConcurrentRuns {
                current: counts.ongoing_runs as u64,
                max: per_user.max_concurrent_runs as u64,
            },
        });
    } else if counts.queued_runs >= per_user.max_queued_runs {
        return Err(Error::RateLimited {
            cluster: cluster.clone(),
            reason: RateLimitReason::QueuedRuns {
                current: counts.queued_runs as u64,
                max: per_user.max_queued_runs as u64,
            },
        });
    }

    let since = Utc::now() - Duration::hours(1);
    let started_in_last_hour = TestRun::started_since(email, cluster, since, conn).await? as usize;

    if started_in_last_hour >= per_user.max_runs_per_hour {
        return Err(Error::RateLimited {
            cluster: cluster.clone(),
            reason: RateLimitReason::RunsPerHour {
                current: started_in_last_hour as u64,
                max: per_user.max_runs_per_hour as u64,
            },
        });
    }

    Ok(counts.queued_executions)
}

#[expect(clippy::large_enum_variant)]
enum PreparedOrGitHub {
    Prepared(PreparedPayload),
    GitHub(GitHubPayload),
}

struct KnownTestPlanLink {
    known_test_plan_id: i32,
    git_sha: Option<String>,
}

struct PayloadWithMeta {
    cluster: ClusterId,
    payload: PreparedOrGitHub,
    link: Option<KnownTestPlanLink>,
}

impl PayloadWithMeta {
    async fn resolve(
        trigger_payload: TriggerPayload,
        default_cluster: &ClusterId,
        conn: &mut PgConnection,
    ) -> Result<Self, Error> {
        let from_known = |known: KnownTestPlan, git_ref: Option<String>, variables| {
            let pinned = known.pinned_workload_cluster();
            let cluster = pinned.clone().unwrap_or_else(|| default_cluster.clone());
            let known_link = KnownTestPlanLink {
                known_test_plan_id: known.id(),
                git_sha: git_ref.clone(),
            };
            let payload = GitHubPayload {
                org: known.org().to_owned(),
                repo: known.repo().to_owned(),
                path: known.path().to_owned(),
                git_ref,
                variables,
            };

            Self {
                cluster,
                payload: PreparedOrGitHub::GitHub(payload),
                link: Some(known_link),
            }
        };

        Ok(match trigger_payload {
            TriggerPayload::Prepared(payload) => Self {
                cluster: default_cluster.clone(),
                link: None,
                payload: PreparedOrGitHub::Prepared(payload),
            },

            TriggerPayload::GitHub(payload) => Self {
                cluster: default_cluster.clone(),
                link: None,
                payload: PreparedOrGitHub::GitHub(payload),
            },

            TriggerPayload::KnownTestPlanUuid(kp) => {
                info!("attempting to look up a known test plan by UUID");
                let known = KnownTestPlan::get_by_uuid(&kp.test_plan_uuid, conn)
                    .await?
                    .ok_or_else(|| Error::UnknownTestPlan {
                        identifier: kp.test_plan_uuid.to_string(),
                    })?;

                from_known(known, kp.git_ref, kp.variables)
            }

            TriggerPayload::KnownTestPlanName(kp) => {
                info!("attempting to look up a known test plan by name");
                let known = KnownTestPlan::get_by_name(&kp.test_plan_name, conn)
                    .await?
                    .ok_or_else(|| Error::UnknownTestPlan {
                        identifier: kp.test_plan_name.clone(),
                    })?;

                from_known(known, kp.git_ref, kp.variables)
            }
        })
    }

    async fn finish(
        self,
        cfg: &Config,
    ) -> Result<(ClusterId, Option<KnownTestPlanLink>, PreparedPayload), Error> {
        Ok(match self.payload {
            PreparedOrGitHub::Prepared(payload) => (self.cluster, self.link, payload),

            PreparedOrGitHub::GitHub(payload) => {
                info!("attempting to pull test plan details from GitHub");
                (
                    self.cluster,
                    self.link,
                    payload.into_prepared(cfg.server_context()).await?,
                )
            }
        })
    }
}

async fn init_run_and_build_summary(
    name: &str,
    variables: Option<Value>,
    initiated_by: Option<String>,
    workload_cluster: ClusterId,
) -> Result<(TestRun, TestRunSummary), Error> {
    let conn = conn!();
    let test_run = TestRun::init(
        name,
        variables,
        initiated_by.as_deref(),
        &workload_cluster,
        conn,
    )
    .await?;
    let summary = test_run
        .clone()
        .try_into_summary_with_executions(conn)
        .await?;

    Ok((test_run, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestServerState;
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::status::Status;
    use uuid::Uuid;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_rejects_invalid_compose_file_provider_usage() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let payload = tss.minimal_invalid_compose_trigger_payload();

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;

        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
        assert!(
            tss.resolver_rx.is_empty(),
            "should not have submitted the test plan"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_initialises_a_new_run_and_submits_to_resolver() -> anyhow::Result<()> {
        let mut tss = TestServerState::new();
        let payload = tss.minimal_trigger_payload();

        // The request itself should return 200
        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .add_header(
                "x-goog-authenticated-user-email",
                "accounts.google.com:ci@my-project.iam.gserviceaccount.com",
            )
            .json(&payload)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        // A test plan should have been submitted to the resolver
        let res = tss.resolver_rx.try_recv();
        assert!(res.is_ok(), "{res:?}");

        // The run should be initialising
        let summary: TestRunSummary = resp.json();
        assert_eq!(summary.current_status, Status::Initialising);
        assert_eq!(
            summary.initiated_by,
            "ci@my-project.iam.gserviceaccount.com"
        );

        // The run should be in the DB, with the initiator captured from the IAP header
        let maybe_run = TestRun::get_by_uuid(&summary.id, conn!()).await.unwrap();
        let run = maybe_run.expect("test run ID did not map to a known run in the DB");
        assert_eq!(
            run.initiated_by(),
            Some("ci@my-project.iam.gserviceaccount.com")
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_defaults_new_run_to_the_configured_default_cluster() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let payload = tss.minimal_trigger_payload();

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let summary: TestRunSummary = resp.json();
        let tr = TestRun::get_by_uuid(&summary.id, conn!())
            .await?
            .expect("test run should be in the DB");

        assert_eq!(tr.workload_cluster().as_str(), "alpha");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_records_unknown_when_the_iap_header_is_absent() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let payload = tss.minimal_trigger_payload();

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let summary: TestRunSummary = resp.json();
        assert_eq!(summary.initiated_by, "unknown");

        let tr = TestRun::get_by_uuid(&summary.id, conn!())
            .await?
            .expect("test run should be in the DB");
        assert_eq!(tr.initiated_by(), None);

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_service_unavailable_when_queue_is_full() -> anyhow::Result<()> {
        let mut cfg = Config::get().clone();
        cfg.workload_clusters.max_queued_executions = 0;

        let tss = TestServerState::new_with_config(&cfg);
        let payload = tss.minimal_trigger_payload();

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .json(&payload)
            .await;

        assert_eq!(resp.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            tss.resolver_rx.is_empty(),
            "should not have submitted the test plan"
        );

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn init_run_and_build_summary_persists_the_resolved_workload_cluster() -> Result<(), Error>
    {
        let (test_run, _) =
            init_run_and_build_summary("test", None, None, ClusterId::new("router_perf")).await?;

        assert_eq!(test_run.workload_cluster().as_str(), "router_perf");

        let fetched = TestRun::get_by_uuid(&test_run.uuid(), conn!())
            .await
            .unwrap()
            .expect("test run should be in the DB");
        assert_eq!(fetched.workload_cluster().as_str(), "router_perf");

        Ok(())
    }

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_rejects_when_max_runs_per_hour_exceeded() -> anyhow::Result<()> {
        let tss = TestServerState::new();
        let payload = tss.minimal_trigger_payload();
        let bare_email = format!(
            "rate-limited-{}@my-project.iam.gserviceaccount.com",
            Uuid::new_v4()
        );
        let header_email = format!("accounts.google.com:{bare_email}");

        // The default per-user limit is 10 runs/hour, so this pushes the user right up to it.
        for _ in 0..10 {
            TestRun::init(
                "prior",
                None,
                Some(&bare_email),
                &ClusterId::new("alpha"),
                conn!(),
            )
            .await?;
        }

        let resp = tss
            .test_server
            .post("/test-run/trigger")
            .add_header("x-goog-authenticated-user-email", header_email)
            .json(&payload)
            .await;

        assert_eq!(resp.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            tss.resolver_rx.is_empty(),
            "should not have submitted the test plan"
        );

        let body: serde_json::Value = resp.json();
        assert_eq!(body["reason"]["kind"], "runs_per_hour");
        assert_eq!(body["reason"]["current"], 10);
        assert_eq!(body["reason"]["max"], 10);
        assert_eq!(body["cluster"], "alpha");

        Ok(())
    }
}
