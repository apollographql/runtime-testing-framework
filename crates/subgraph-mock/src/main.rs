use clap::Parser;
use hyper::service::service_fn;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use std::net::SocketAddr;
use subgraph_mock::{Args, handle::handle_request};
use tokio::net::TcpListener;
use tracing::{Level, error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_max_level(Level::INFO)
        .try_init()
        .expect("unable to set a global tracing subscriber");

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
