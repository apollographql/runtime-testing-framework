//! Summary details for a registered test plan: the variables a run can be triggered with, the
//! services each execution deploys, and how previous runs have gone.
//!
//! See the comment on `build_details_without_history` for more information around caching
//! behaviour.
use crate::{
    Error, Result,
    config::Config,
    conn,
    db::{ClusterId, KnownTestPlan},
    state::ServerState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use cached::cached;
use chrono::Utc;
use rtf_config::context::ResolutionContext;
use rtf_integrations::github::Client as _;
use rtf_orchestrator_shared::{
    test_plan::OrchestratorTestPlan,
    test_plan_details::{
        EnvironmentSummary, MatrixSummary, TestPlanDetails, TestPlanDetailsParams, TestPlanHistory,
        TestPlanSource, TestPlanVariable,
    },
};
use uuid::Uuid;

pub async fn handler(
    State(ServerState { eq_state, .. }): State<ServerState>,
    Path(uuid): Path<Uuid>,
    Query(params): Query<TestPlanDetailsParams>,
) -> Result<Json<TestPlanDetails>> {
    let conn = conn!();
    let known = KnownTestPlan::get_by_uuid(&uuid, conn)
        .await?
        .ok_or_else(|| Error::UnknownTestPlan {
            identifier: uuid.to_string(),
        })?;

    let history = known
        .test_plan_history(params.history_window(Utc::now()), conn)
        .await?;
    let cluster = known
        .pinned_workload_cluster()
        .unwrap_or_else(|| eq_state.default_cluster().clone());

    build_details(
        &known,
        params.git_ref,
        cluster,
        history,
        Config::get().server_context(),
    )
    .await
    .map(Json)
}

async fn build_details(
    known: &KnownTestPlan,
    git_ref: Option<String>,
    cluster: ClusterId,
    history: TestPlanHistory,
    ctx: impl ResolutionContext,
) -> Result<TestPlanDetails> {
    // Resolving the ref up front pins every file we go on to read to a single commit, rather than
    // racing a branch that could move mid-request.
    let sha = ctx
        .github_client()
        .expect("to have a github client")
        .commit_sha(known.org(), known.repo(), git_ref.as_deref())
        .await?;

    let mut details = build_details_without_history(known, git_ref, sha, ctx).await?;
    details.cluster = cluster.to_string();
    details.history = history;

    Ok(details)
}

// We cache the details data pulled from GitHub based on the SHA the test plan was pulled from and
// the Test Plan UUID. DB data is pulled every time as it changes more frequently and the queries
// against our own DB are cheap. The max size here is effectively arbitrary but in place to avoid
// unbounded growth of the cache if we end up registering a large number of test plans.
#[cached(
    max_size = 50,
    key = "String",
    convert = r#"{ format!("{}-{sha}", known.uuid()) }"#
)]
async fn build_details_without_history(
    known: &KnownTestPlan,
    git_ref: Option<String>,
    sha: String,
    mut ctx: impl ResolutionContext,
) -> Result<TestPlanDetails> {
    let (org, repo, path) = (known.org(), known.repo(), known.path());
    let (test_plan, sources) = OrchestratorTestPlan::try_load_and_resolve_from_github(
        org,
        repo,
        path,
        Some(sha.clone()),
        &ctx,
    )
    .await?;

    ctx.set_sources(sources);

    Ok(TestPlanDetails {
        uuid: known.uuid(),
        name: known.name().to_string(),
        description: known.description(),
        source: TestPlanSource {
            org: org.to_string(),
            repo: repo.to_string(),
            path: path.to_string(),
            git_ref,
            sha,
        },
        variables: TestPlanVariable::from_test_plan(&test_plan),
        matrix: MatrixSummary::new(&test_plan.matrix),
        environment: EnvironmentSummary::try_from_test_plan(&test_plan, &ctx).await?,
        // set from DB state in build_details
        cluster: Default::default(),
        history: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TestServerState;
    use reqwest::StatusCode;

    #[cfg_attr(not(feature = "db_tests"), ignore)]
    #[tokio::test]
    async fn handler_returns_404_for_an_unknown_test_plan() -> anyhow::Result<()> {
        let tss = TestServerState::new();

        let resp = tss
            .test_server
            .get(&format!("/test-plan/{}/details", Uuid::new_v4()))
            .await;

        assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);

        Ok(())
    }
}
