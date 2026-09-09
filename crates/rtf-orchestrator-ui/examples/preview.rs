//! Preview how each page template is rendering without requiring a running orchestrator to provide
//! the data.
//!
//! Run with:
//!   cargo run --example preview --features preview
use askama::Template;
use axum::{
    Router,
    extract::Path,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use rtf_orchestrator_ui::preview;
use rust_embed::RustEmbed;
use tokio::net::TcpListener;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

const PAGES: &[(&str, &str)] = &[
    ("/ui", "Index — run search + recent runs"),
    (
        "/ui/run/running",
        "Run — in progress, polling, with trigger variables",
    ),
    ("/ui/run/terminal", "Run — completed"),
    ("/ui/run/not-found", "Run — not found"),
    (
        "/ui/execution/with-parent",
        "Execution — linked to a parent run",
    ),
    ("/ui/execution/without-parent", "Execution — no parent run"),
    ("/ui/execution/not-found", "Execution — not found"),
    ("/ui/test-plans", "Known test plans — list"),
    (
        "/ui/test-plan/detail",
        "Known test plan — detail + trigger form (docker-compose environment)",
    ),
    (
        "/ui/test-plan/detail-k8s",
        "Known test plan — detail + trigger form (k8s environment)",
    ),
    ("/ui/test-plan/not-found", "Known test plan — not found"),
    ("/ui/trigger", "Trigger a run from GitHub — empty form"),
    (
        "/ui/trigger/error",
        "Trigger a run from GitHub — resubmitted with an error",
    ),
    ("/ui/error", "Generic error page"),
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(nav))
        .route("/ui", get(|| async { render(preview::index()) }))
        .route(
            "/ui/run/running",
            get(|| async { render(preview::run_running()) }),
        )
        .route(
            "/ui/run/terminal",
            get(|| async { render(preview::run_terminal()) }),
        )
        .route(
            "/ui/run/not-found",
            get(|| async { render(preview::run_not_found()) }),
        )
        .route(
            "/ui/execution/with-parent",
            get(|| async { render(preview::execution_with_parent()) }),
        )
        .route(
            "/ui/execution/without-parent",
            get(|| async { render(preview::execution_without_parent()) }),
        )
        .route(
            "/ui/execution/not-found",
            get(|| async { render(preview::execution_not_found()) }),
        )
        .route(
            "/ui/test-plans",
            get(|| async { render(preview::test_plans()) }),
        )
        .route(
            "/ui/test-plan/detail",
            get(|| async { render(preview::test_plan_detail()) }),
        )
        .route(
            "/ui/test-plan/detail-k8s",
            get(|| async { render(preview::test_plan_detail_k8s()) }),
        )
        .route(
            "/ui/test-plan/not-found",
            get(|| async { render(preview::test_plan_not_found()) }),
        )
        .route(
            "/ui/trigger",
            get(|| async { render(preview::trigger_empty()) }),
        )
        .route(
            "/ui/trigger/error",
            get(|| async { render(preview::trigger_error()) }),
        )
        .route("/ui/error", get(|| async { render(preview::error_page()) }))
        .route("/ui/static/{*path}", get(serve_asset));

    let addr = "127.0.0.1:8080";
    let listener = TcpListener::bind(addr).await?;
    println!("preview server listening on http://{addr}/");

    axum::serve(listener, app).await?;

    Ok(())
}

fn render<T: Template>(template: T) -> Html<String> {
    Html(
        template
            .render()
            .expect("preview fixtures should always render"),
    )
}

async fn serve_asset(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => (
            [(header::CONTENT_TYPE, file.metadata.mimetype())],
            file.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn nav() -> Html<String> {
    let links: String = PAGES
        .iter()
        .map(|(path, label)| format!(r#"<li><a href="{path}">{label}</a></li>"#))
        .collect();

    Html(format!(
        "<html><body><h1>rtf-orchestrator-ui preview</h1><ul>{links}</ul></body></html>"
    ))
}
