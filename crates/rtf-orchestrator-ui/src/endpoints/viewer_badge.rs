use crate::{
    endpoints::{render_body, to_response},
    orchestrator::{Client, OrchestratorClient},
    templates::ViewerBadgeTemplate,
    view::ViewerBadge,
};
use axum::response::Response;
use reqwest::StatusCode;
use tracing::error;

pub async fn handler<C: Client>(OrchestratorClient(client): OrchestratorClient<C>) -> Response {
    let badge = match client.whoami().await {
        Ok(whoami) => ViewerBadge::for_whoami(&whoami),
        Err(error) => {
            error!(%error, "failed to identify the current user from orchestrator");
            None
        }
    };

    to_response(render_body(StatusCode::OK, ViewerBadgeTemplate { badge }))
}
