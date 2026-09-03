//! Queue management for purging known "bad" / stuck runs and executions.
use crate::{
    Error, Result, conn,
    db::{TestExecution, TestRun},
    endpoints::AdminUser,
    state::ServerState,
};
use axum::{
    Json,
    extract::{Path, State},
};
use rtf_orchestrator_shared::summary::{TestExecutionSummary, TestRunSummary};
use tracing::warn;
use uuid::Uuid;

pub async fn ex_handler(
    AdminUser(email): AdminUser,
    Path(ex_id): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
) -> Result<Json<TestExecutionSummary>> {
    let conn = conn!();
    let ex = TestExecution::get_by_uuid(&ex_id, conn)
        .await?
        .ok_or(Error::UnknownTestExecution { id: ex_id })?;

    warn!(%ex_id, "marking execution as cancelled and purging queue state");
    eq_state.purge_execution(ex.clone(), email, conn).await;

    Ok(Json(ex.try_into_summary_with_status_history(conn).await?))
}

pub async fn run_handler(
    AdminUser(email): AdminUser,
    Path(run_id): Path<Uuid>,
    State(ServerState { eq_state, .. }): State<ServerState>,
) -> Result<Json<TestRunSummary>> {
    let conn = conn!();
    let tr = TestRun::get_by_uuid(&run_id, conn)
        .await?
        .ok_or(Error::UnknownTestRun { id: run_id })?;

    warn!(%run_id, "marking run as cancelled and purging queue state");
    eq_state.purge_run(tr.clone(), email, conn).await;

    Ok(Json(tr.try_into_summary_with_executions(conn).await?))
}
