use axum::{
    Router,
    routing::{get, post},
    serve,
};
use tokio::net::TcpListener;
use tracing::info;

pub mod config;
pub mod db;
pub mod endpoints;
pub mod error;
pub mod resolver;
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

    let (state, rx) = ServerState::new();
    tokio::spawn(resolver::resolver_task(rx));

    info!("starting axum server");
    let routes = build_routes(state);
    let listener = TcpListener::bind(cfg.socket_addr()).await.unwrap();

    serve(listener, routes).await?;

    Ok(())
}

fn build_routes(state: ServerState) -> Router {
    use endpoints::{execution_status, health, run_status, trigger};

    Router::new()
        .route("/health", get(health::handler))
        .route(
            "/test-execution/{id}/status",
            get(execution_status::handler),
        )
        .route("/test-run/{id}/status", get(run_status::handler))
        .route("/test-run/trigger", post(trigger::handler))
        .with_state(state)
}
