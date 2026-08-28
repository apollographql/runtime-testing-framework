#![warn(clippy::undocumented_unsafe_blocks)]
use crate::gcs::GCSClient;
use axum::{
    Extension, Router,
    extract::DefaultBodyLimit,
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
pub mod iap_identity;
pub mod k8s;
pub mod rate_limit;
pub mod resolver;
pub mod state;

pub use error::{Error, Result};

use config::Config;
use db::pool::check_db_conn;
use event_loop::EventQueue;
use state::ServerState;

pub async fn run_server(reload_handle: Handle<EnvFilter, Registry>) -> error::Result<()> {
    info!("Loading config from file");
    let cfg = Config::get();

    info!("Checking database connection");
    check_db_conn().await?;

    let (mut event_queue, prov_handle, eq_state, rx) = EventQueue::new(&cfg.workload_clusters);

    info!("Initialising event queue state");
    event_queue.init_queue_state(cfg, conn!()).await?;

    let gcs_client = GCSClient::new_from_config(cfg).await?;
    let state = ServerState::new(eq_state, gcs_client);

    tokio::spawn(resolver::resolver_task(rx, prov_handle));
    tokio::spawn(event_loop::event_loop_task(event_queue));

    info!("starting axum server");
    let routes = build_routes(cfg, state, Some(reload_handle));
    let listener = TcpListener::bind(cfg.socket_addr()).await.unwrap();

    serve(listener, routes).await?;

    Ok(())
}

fn build_routes(
    cfg: &Config,
    state: ServerState,
    reload_handle: Option<Handle<EnvFilter, Registry>>,
) -> Router {
    use endpoints::{
        admin, execution_artifacts, execution_config, execution_status, generate_upload_urls,
        health, known_test_plan_cluster_pin, known_test_plans, list_runs, register_known_test_plan,
        run_status, test_plan_details, trigger, whoami,
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
            "/test-execution/{id}/output-config",
            get(execution_config::output_handler),
        )
        .route(
            "/test-execution/{id}/status",
            get(execution_status::get_handler).post(execution_status::post_handler),
        )
        .route("/test-plan", get(known_test_plans::list_handler))
        .route("/test-plan/{uuid}", get(known_test_plans::by_uuid_handler))
        .route("/test-plan/{uuid}/details", get(test_plan_details::handler))
        .route(
            "/test-plan/{uuid}/runs",
            get(list_runs::known_test_plan_handler),
        )
        .route(
            "/test-plan/register",
            post(register_known_test_plan::handler),
        )
        .route(
            "/test-plan/{uuid}/pinned-cluster",
            post(known_test_plan_cluster_pin::set_handler)
                .delete(known_test_plan_cluster_pin::clear_handler),
        )
        .route("/test-run", get(list_runs::handler))
        .route("/test-run/{id}/status", get(run_status::handler))
        .route("/test-run/trigger", post(trigger::handler))
        .route("/whoami", get(whoami::handler))
        .with_state(state)
        .layer(DefaultBodyLimit::max(
            cfg.server.body_limit_mb * 1024 * 1024,
        ));

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
                None,
            )
        }

        pub fn new_with_gcs_client(gcs_client: GCSClient) -> Self {
            Self::new_with_params(Config::get(), gcs_client, None)
        }

        pub fn new_with_config(cfg: &Config) -> Self {
            Self::new_with_params(
                cfg,
                GCSClient::new_mock("internal_url", "public_url", "bucket", None),
                None,
            )
        }

        pub fn new_with_admins(admins: &[&str]) -> Self {
            Self::new_with_params(
                Config::get(),
                GCSClient::new_mock("internal_url", "public_url", "bucket", None),
                Some(admins),
            )
        }

        pub fn new_with_config_and_admins(cfg: &Config, admins: &[&str]) -> Self {
            Self::new_with_params_and_clusters(
                cfg,
                GCSClient::new_mock("internal_url", "public_url", "bucket", None),
                Some(admins),
            )
        }

        pub fn new_with_params(
            cfg: &Config,
            gcs_client: GCSClient,
            admins: Option<&[&str]>,
        ) -> Self {
            Self::new_with_params_and_clusters(cfg, gcs_client, admins)
        }

        pub fn new_with_params_and_clusters(
            cfg: &Config,
            gcs_client: GCSClient,
            admins: Option<&[&str]>,
        ) -> Self {
            let (_, prov_handle, eq_state, resolver_rx) = EventQueue::new(&cfg.workload_clusters);

            let mut state = ServerState::new(eq_state, gcs_client);
            if let Some(admins) = admins {
                state.test_admins = Some(admins.iter().map(|s| s.to_string()).collect());
            }

            let test_server = TestServer::new(build_routes(cfg, state.clone(), None));

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
