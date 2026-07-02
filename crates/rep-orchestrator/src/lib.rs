#![warn(clippy::undocumented_unsafe_blocks)]
use crate::gcs::GCSClient;
use axum::{
    Extension, Router,
    routing::{get, post},
    serve,
};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{EnvFilter, Registry, reload::Handle};

pub mod config;
pub mod context;
pub mod db;
pub mod endpoints;
pub mod error;
pub mod event_loop;
pub mod gcs;
pub mod k8s;
pub mod resolver;
pub mod state;

pub use error::{Error, Result};

use config::Config;
use db::pool::check_db_conn;
use event_loop::EventQueue;
use state::ServerState;

pub async fn run_server(reload_handle: Handle<EnvFilter, Registry>) -> error::Result<()> {
    info!("Loading config from environment");
    let cfg = Config::get();

    info!("Checking database connection");
    check_db_conn().await?;

    let (mut event_queue, prov_handle, eq_state, rx) =
        EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

    info!("Initialising event queue state");
    event_queue.init_queue_state(cfg, conn!()).await?;

    let gcs_client = GCSClient::new_from_config(cfg).await?;
    let state = ServerState::new(eq_state, gcs_client);

    tokio::spawn(resolver::resolver_task(rx, prov_handle));
    tokio::spawn(event_loop::event_loop_task(event_queue));

    info!("starting axum server");
    let routes = build_routes(state, Some(reload_handle));
    let listener = TcpListener::bind(cfg.socket_addr()).await.unwrap();

    serve(listener, routes).await?;

    Ok(())
}

fn build_routes(state: ServerState, reload_handle: Option<Handle<EnvFilter, Registry>>) -> Router {
    use endpoints::{
        admin, execution_artifacts, execution_config, execution_status, generate_upload_urls,
        health, run_status, trigger,
    };

    let mut router = Router::new()
        .route(
            "/admin/event-queue-snapshot",
            get(admin::event_queue_snapshot_handler),
        )
        .route("/health", get(health::handler))
        .route(
            "/test-execution/{id}/generate-upload-urls",
            post(generate_upload_urls::handler),
        )
        .route(
            "/test-execution/{id}/log.txt",
            get(execution_artifacts::log_file_handler),
        )
        .route(
            "/test-execution/{id}/output.zip",
            get(execution_artifacts::output_zip_handler),
        )
        .route(
            "/test-execution/{id}/environment-config",
            get(execution_config::env_handler),
        )
        .route(
            "/test-execution/{id}/scenario-config",
            get(execution_config::scenario_handler),
        )
        .route(
            "/test-execution/{id}/prometheus-queries",
            get(execution_config::prometheus_handler),
        )
        .route(
            "/test-execution/{id}/status",
            get(execution_status::get_handler).post(execution_status::post_handler),
        )
        .route("/test-run/{id}/status", get(run_status::handler))
        .route("/test-run/trigger", post(trigger::handler))
        .with_state(state);

    if let Some(reload_handle) = reload_handle {
        router = router.route(
            "/admin/logging-filter",
            get(admin::get_logging_filter_handler)
                .post(admin::set_logging_filter_handler)
                .layer(Extension(reload_handle)),
        );
    }

    router
}

#[cfg(test)]
mod test_helpers {
    use super::*;
    use crate::{event_loop::ProvisioningHandle, resolver::ResolverInput};
    use axum_test::TestServer;
    use tokio::sync::mpsc::UnboundedReceiver;

    /// A wrapper around the top level state needed for writing tests of the overall server
    /// behaviour using [axum_test](https://docs.rs/axum-test/latest/axum_test/).
    pub struct TestServerState {
        pub test_server: TestServer,
        pub prov_handle: ProvisioningHandle,
        pub state: ServerState,
        pub resolver_rx: UnboundedReceiver<ResolverInput>,
    }

    impl TestServerState {
        pub fn new() -> Self {
            Self::new_with_params(
                Config::get(),
                GCSClient::new_mock("internal_url", "public_url", "bucket", None),
            )
        }

        pub fn new_with_gcs_client(gcs_client: GCSClient) -> Self {
            Self::new_with_params(Config::get(), gcs_client)
        }

        pub fn new_with_config(cfg: &Config) -> Self {
            Self::new_with_params(
                cfg,
                GCSClient::new_mock("internal_url", "public_url", "bucket", None),
            )
        }

        pub fn new_with_params(cfg: &Config, gcs_client: GCSClient) -> Self {
            let (_, prov_handle, eq_state, resolver_rx) =
                EventQueue::new(cfg.max_concurrent_executions, cfg.max_queued_executions);

            let state = ServerState::new(eq_state, gcs_client);
            let test_server = TestServer::new(build_routes(state.clone(), None));

            Self {
                test_server,
                prov_handle,
                state,
                resolver_rx,
            }
        }

        pub fn minimal_trigger_payload(&self) -> serde_json::Value {
            serde_json::from_str(include_str!("../resources/trigger-payloads/minimal.json"))
                .unwrap()
        }

        pub fn minimal_invalid_compose_trigger_payload(&self) -> serde_json::Value {
            serde_json::from_str(include_str!(
                "../resources/trigger-payloads/invalid-compose.json"
            ))
            .unwrap()
        }
    }
}
