use crate::{config::Config, links::LinksConfig};
use axum::{
    Extension, Router,
    routing::{get, post},
};
use tokio::net::TcpListener;
use tracing::info;

pub mod config;
pub mod preview;

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
        "starting rtf-orchestrator-ui"
    );

    let client = orchestrator::HttpClient::try_new(cfg.orchestrator_url.clone())?;
    let links_cfg = LinksConfig::from(cfg);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router(client, links_cfg)).await?;

    Ok(())
}

fn router<C>(orchestrator_client: C, links_cfg: LinksConfig) -> Router
where
    C: orchestrator::Client + Clone,
{
    use endpoints::{
        execution_detail, execution_log, execution_output_zip, health, index, run_status,
        test_plan_detail, test_plans, trigger,
    };

    Router::new()
        .route("/ui", get(index::handler::<C>))
        .route("/ui/health", get(health))
        .route("/ui/run/{id}", get(run_status::handler::<C>))
        .route("/ui/test-plans", get(test_plans::handler::<C>))
        .route("/ui/test-plan/{uuid}", get(test_plan_detail::handler::<C>))
        .route(
            "/ui/test-plan/{uuid}/trigger",
            post(test_plan_detail::post_trigger::<C>),
        )
        .route("/ui/execution/{eid}", get(execution_detail::handler::<C>))
        .route(
            "/ui/execution/{eid}/log.txt",
            get(execution_log::handler::<C>),
        )
        .route(
            "/ui/execution/{eid}/output.zip",
            get(execution_output_zip::handler::<C>),
        )
        .route(
            "/ui/trigger",
            get(trigger::get_handler).post(trigger::post_handler::<C>),
        )
        .route("/ui/static/{*path}", get(assets::serve))
        .layer(Extension(links_cfg))
        .with_state(orchestrator_client)
}
