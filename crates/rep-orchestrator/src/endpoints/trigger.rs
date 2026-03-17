use crate::{conn, db::TestRun, error::Error, response_types::TestRunSummary, state::ServerState};
use axum::{Json, extract::State};
use rtf_config::formats::RepPayload;

pub async fn handler(
    State(state): State<ServerState>,
    Json(payload): Json<RepPayload>,
) -> Result<Json<TestRunSummary>, Error> {
    let conn = conn!();
    let test_run = TestRun::init(&payload.test_plan.name, conn).await?;

    let summary = test_run.clone().try_into_summary(conn).await?;

    state
        .submit_test_plan(test_run, payload)
        .map_err(|_| Error::ResolverChannelClosed)?;

    Ok(Json(summary))
}
