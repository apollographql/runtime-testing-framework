//! Summary details for a registered test plan: the variables a run can be triggered with, the
//! services each execution deploys, and how previous runs have gone.
use crate::{Error, Result, config::Config, conn, db::KnownTestPlan};
use axum::{
    Json,
    extract::{Path, Query},
};
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

    build_details(
        &known,
        params.git_ref,
        history,
        Config::get().server_context(),
    )
    .await
    .map(Json)
}

async fn build_details(
    known: &KnownTestPlan,
    git_ref: Option<String>,
    history: TestPlanHistory,
    mut ctx: impl ResolutionContext,
) -> Result<TestPlanDetails> {
    let (org, repo, path) = (known.org(), known.repo(), known.path());

    // Resolving the ref up front pins every file we go on to read to a single commit, rather than
    // racing a branch that could move mid-request.
    let sha = ctx
        .github_client()
        .expect("to have a github client")
        .commit_sha(org, repo, git_ref.as_deref())
        .await?;

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
        history,
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
