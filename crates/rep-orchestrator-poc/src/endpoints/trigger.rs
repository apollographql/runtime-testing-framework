//! Trigger a new test run
use crate::{AppError, rep_test_plan::RepTestPlan, state::ServerState, test_run::TestRunSummary};
use axum::{Json, extract::State};
use tracing::{error, info};
use uuid::Uuid;

pub async fn handler(
    State(state): State<ServerState>,
    Json(rtp): Json<RepTestPlan>,
) -> Result<Json<TestRunSummary>, AppError> {
    inner(rtp, state).await.map_err(AppError)
}

async fn inner(rtp: RepTestPlan, state: ServerState) -> anyhow::Result<Json<TestRunSummary>> {
    let summary = TestRunSummary {
        id: Uuid::new_v4(),
        ..Default::default()
    };

    info!(id=%summary.id, "Submitting test plan for resolution");
    if let Err(rtp) = state.submit_test_plan(summary.id, rtp) {
        error!(id=%summary.id, name=%rtp.test_plan.name, "unable to submit test plan for resolution");
    };

    Ok(Json(summary))
}
