use axum::{
    Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    serve,
};
use std::{collections::HashMap, env, net::SocketAddr, sync::LazyLock};
use tokio::net::TcpListener;
use tracing::info;

pub mod context;
pub mod endpoints;
pub mod rep_test_plan;
pub mod resolver;
pub mod state;
pub mod test_execution;
pub mod test_run;

use state::ServerState;

const DEFAULT_PORT: u16 = 8035;

pub(crate) static ENV_VARS: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| env::vars().collect());

pub async fn run_server(state: ServerState) -> anyhow::Result<()> {
    // Read env vars
    // - port
    // - kubeconfig
    // - github token
    // - db creds
    // - gcp creds?

    // check DB connectivity

    // spawn event loop task

    info!("starting axum server");
    let routes = build_routes(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT));
    let listener = TcpListener::bind(addr).await.unwrap();

    serve(listener, routes).await?;

    Ok(())
}

fn build_routes(state: ServerState) -> Router {
    Router::new()
        .route("/test-run/trigger", post(endpoints::trigger::handler))
        .with_state(state)
}

pub struct AppError(pub anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
