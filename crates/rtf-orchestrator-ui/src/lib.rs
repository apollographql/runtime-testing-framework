use crate::{
    config::Config,
    endpoints::{health, index},
};
use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tracing::info;

pub mod config;

mod assets;
mod endpoints;
mod templates;

pub async fn run_server() -> anyhow::Result<()> {
    let cfg = Config::get();
    let addr = cfg.socket_addr();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        %addr,
        "starting rep-orchestrator-ui"
    );

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router()).await?;

    Ok(())
}

/// Build the UI router, backed by `provider` for run data. All routes live under the `/ui` prefix.
fn router() -> Router {
    Router::new()
        .route("/ui", get(index))
        .route("/ui/health", get(health))
        .route("/ui/static/{*path}", get(assets::serve))
}
