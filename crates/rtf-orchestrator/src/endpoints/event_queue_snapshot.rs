use crate::{Result, state::ServerState};
use axum::{Json, extract::State};
use rtf_orchestrator_shared::event_queue::EventQueueSnapshot;

pub async fn handler(
    State(ServerState { eq_state, .. }): State<ServerState>,
) -> Result<Json<EventQueueSnapshot>> {
    let snapshot = eq_state.event_queue_snapshot().await;

    Ok(Json(snapshot))
}
