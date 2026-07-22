use crate::{
    config::Config,
    endpoints::{execution_detail, execution_log, execution_output_zip, health, index, run_status},
    links::LinksConfig,
};
use axum::{Extension, Router, routing::get};
use tokio::net::TcpListener;
use tracing::info;

pub mod config;

mod assets;
mod endpoints;
mod links;
mod orchestrator;
mod status;
mod templates;
mod view;

pub async fn run_server() -> anyhow::Result<()> {
    let cfg = Config::get();
    let addr = cfg.socket_addr();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        %addr,
        "starting rep-orchestrator-ui"
    );

    let client = orchestrator::HttpClient::try_new(cfg.orchestrator_url.clone())?;
    let links_cfg = LinksConfig::from(cfg);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router(client, links_cfg)).await?;

    Ok(())
}

/// Build the UI router, backed by `provider` for run data. All routes live under the `/ui` prefix.
/// `links_cfg` is shared across handlers via an [Extension] rather than `State`, since `State` is
/// reserved for the generic orchestrator [`orchestrator::Client`].
fn router<C>(orchestrator_client: C, links_cfg: LinksConfig) -> Router
where
    C: orchestrator::Client + Clone,
{
    Router::new()
        .route("/ui", get(index::<C>))
        .route("/ui/health", get(health))
        .route("/ui/run/{id}", get(run_status::<C>))
        .route("/ui/execution/{eid}", get(execution_detail::<C>))
        .route("/ui/execution/{eid}/log.txt", get(execution_log::<C>))
        .route(
            "/ui/execution/{eid}/output.zip",
            get(execution_output_zip::<C>),
        )
        .route("/ui/static/{*path}", get(assets::serve))
        .layer(Extension(links_cfg))
        .with_state(orchestrator_client)
}
