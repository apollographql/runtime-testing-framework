use crate::{
    endpoints::{Content, download_response},
    orchestrator::Client,
};
use axum::{
    extract::{Path, State},
    response::Response,
};
use uuid::Uuid;

/// `GET /ui/execution/{eid}/log.txt`
///
/// View the execution's log in the browser in plaintext.
pub async fn handler<C: Client>(
    State(orchestrator_client): State<C>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    download_response(
        orchestrator_client.execution_log(execution_id).await,
        format!("{execution_id}-log.txt"),
        Content::InlineText,
    )
}
