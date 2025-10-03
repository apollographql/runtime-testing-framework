use clap::Parser;
use hyper::service::service_fn;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use std::{net::SocketAddr, panic::set_hook};
use subgraph_mock::{Args, handle::handle_request};
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt,
    prelude::*,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().json().flatten_event(true).with_target(false))
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .try_init()
        .expect("unable to set a global tracing subscriber");

    set_hook(Box::new(|panic| {
        if let Some(loc) = panic.location() {
            error!(
                message=%panic,
                panic.file=loc.file(),
                panic.line=loc.line(),
                panic.column=loc.column()
            );
        } else {
            error!(message=%panic);
        }
    }));

    let port = Args::parse().init()?;
    let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
    info!(%port, "subgraph mock server now listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::spawn(async move {
            if let Err(err) = Builder::new(TokioExecutor::new())
                .serve_connection(io, service_fn(handle_request))
                .await
            {
                error!(%err, "server error");
            }
        });
    }
}
