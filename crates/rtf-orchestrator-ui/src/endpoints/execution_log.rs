use crate::{
    endpoints::{Content, download_response},
    orchestrator::{Client, OrchestratorClient},
};
use axum::{extract::Path, response::Response};
use uuid::Uuid;

pub async fn handler<C: Client>(
    OrchestratorClient(client): OrchestratorClient<C>,
    Path(execution_id): Path<Uuid>,
) -> Response {
    download_response(
        client.execution_log(execution_id).await,
        format!("{execution_id}-log.txt"),
        Content::InlineText,
    )
}
