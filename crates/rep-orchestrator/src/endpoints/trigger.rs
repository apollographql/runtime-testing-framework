//! Trigger a new test run
use crate::{AppError, context::ServerState, test_run::TestRunSummary};
use axum::{Json, extract::State};
use rtf_config::formats::TestPlanConfig;
use tracing::error;
use uuid::Uuid;

pub async fn handler(
    State(state): State<ServerState>,
    body: String,
) -> Result<Json<TestRunSummary>, AppError> {
    inner(body, state).await.map_err(AppError)
}

async fn inner(body: String, state: ServerState) -> anyhow::Result<Json<TestRunSummary>> {
    let tp: TestPlanConfig = serde_yaml::from_str(&body)?;

    let summary = TestRunSummary {
        id: Uuid::new_v4(),
        ..Default::default()
    };

    if let Err(tp) = state.submit_test_plan(summary.id, tp) {
        error!(id=%summary.id, name=%tp.name, "unable to submit test plan for resolution");
    };

    Ok(Json(summary))
}
