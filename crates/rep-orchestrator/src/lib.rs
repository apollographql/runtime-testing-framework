use axum::{Router, routing::get, serve};
use tokio::net::TcpListener;
use tracing::info;

pub mod config;
pub mod db;
pub mod endpoints;
pub mod error;
pub mod response_types;
pub mod state;

pub use error::{Error, Result};

use config::Config;
use db::pool::check_db_conn;
use state::ServerState;

pub async fn run_server() -> error::Result<()> {
    info!("Loading config from environment");
    let cfg = Config::get();

    info!("Checking database connection");
    check_db_conn().await?;

    let (state, _rx) = ServerState::new();

    info!("starting axum server");
    let routes = build_routes(state);
    let listener = TcpListener::bind(cfg.socket_addr()).await.unwrap();

    serve(listener, routes).await?;

    Ok(())
}

fn build_routes(state: ServerState) -> Router {
    use endpoints::{execution_status, run_status};

    Router::new()
        .route(
            "/test-execution/{id}/status",
            get(execution_status::handler),
        )
        .route("/test-run/{id}/status", get(run_status::handler))
        .with_state(state)
}
