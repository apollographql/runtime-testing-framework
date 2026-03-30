use axum::{
    Router,
    routing::{get, post},
    serve,
};
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};
use tracing::info;

pub mod config;
pub mod context;
pub mod db;
pub mod endpoints;
pub mod error;
pub mod event_loop;
pub mod k8s;
pub mod resolver;
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
    let (etx, erx) = unbounded_channel::<event_loop::Event>();

    tokio::spawn(resolver::resolver_task(rx, etx.clone()));
    tokio::spawn(event_loop::event_loop_task(etx, erx));

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
            get(execution_status::get_handler).post(execution_status::post_handler),
        )
        .route("/test-run/{id}/status", get(run_status::handler))
        .route("/test-run/trigger", post(trigger::handler))
        .with_state(state)
}

#[cfg(test)]
mod test_helpers {
    use super::*;
    use crate::state::TestRunWithPayload;
    use axum_test::TestServer;
    use tokio::sync::mpsc::UnboundedReceiver;

    /// A wrapper around the top level state needed for writing tests of the overall server
    /// behaviour using [axum_test](https://docs.rs/axum-test/latest/axum_test/).
    pub struct TestServerState {
        pub test_server: TestServer,
        pub resolver_rx: UnboundedReceiver<TestRunWithPayload>,
    }

    impl TestServerState {
        pub fn new() -> Self {
            let (state, resolver_rx) = ServerState::new();
            let test_server = TestServer::new(build_routes(state));

            Self {
                test_server,
                resolver_rx,
            }
        }

        pub fn minimal_trigger_payload(&self) -> serde_json::Value {
            serde_json::from_str(include_str!("../resources/trigger-payloads/minimal.json"))
                .unwrap()
        }
    }
}
