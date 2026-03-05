use axum::{
    Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    serve,
};
use std::net::SocketAddr;
use tokio::net::TcpListener;

pub mod endpoints;

pub async fn run_server() -> anyhow::Result<()> {
    // Read env vars
    // - port
    // - kubeconfig
    // - github token
    // - db creds
    // - gcp creds?

    // check DB connectivity

    // spawn event loop task
    // spawn test plan resolver task

    let routes = build_routes();
    let port = 8035; // will need to be read from env var
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Start axum server
    let listener = TcpListener::bind(addr).await.unwrap();
    serve(listener, routes).await?;

    Ok(())
}

fn build_routes() -> Router {
    Router::new().route("/test-run/trigger", post(endpoints::trigger::handler))
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
