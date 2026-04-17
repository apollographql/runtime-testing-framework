//! Minimal in-memory GCS mock server for local integration testing of the rep-orchestrator crate.
use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    routing::put,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use tracing::info;

type Store = Arc<Mutex<HashMap<String, Bytes>>>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let app = Router::new()
        .route("/{*path}", put(put_handler).get(get_handler))
        .with_state(Store::default());
    let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
    let listener = TcpListener::bind(addr).await.unwrap();

    info!(%addr, "listening on");
    axum::serve(listener, app).await.unwrap();
}

async fn put_handler(
    Path(path): Path<String>,
    State(store): State<Store>,
    body: Bytes,
) -> StatusCode {
    info!(%path, bytes = body.len(), "PUT");
    store.lock().unwrap().insert(path, body);

    StatusCode::OK
}

async fn get_handler(Path(path): Path<String>, State(store): State<Store>) -> (StatusCode, Bytes) {
    info!(%path, "GET");
    match store.lock().unwrap().get(&path).cloned() {
        Some(bytes) => (StatusCode::OK, bytes),
        None => (StatusCode::NOT_FOUND, Bytes::from("not found")),
    }
}
