use rep_orchestrator::run_server;
use rustls::crypto::aws_lc_rs;
use std::{io::stdout, process};
use tracing::{error, info, subscriber::set_global_default};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, registry, reload};

#[tokio::main]
async fn main() {
    let fmt_layer = fmt::layer()
        .with_writer(stdout)
        .json()
        .flatten_event(true)
        .with_span_list(true)
        .with_current_span(false);

    let (reload_layer, reload_handle) = reload::Layer::new(EnvFilter::from_default_env());
    let subscriber = registry().with(reload_layer).with(fmt_layer);

    set_global_default(subscriber).expect("unable to set a global tracing subscriber");

    if aws_lc_rs::default_provider().install_default().is_err() {
        panic!("unable to install default crypto provider");
    }

    info!(
        "starting server version={}-{}",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_SHA")
    );

    if let Err(error) = run_server(reload_handle).await {
        error!(%error, "Fatal error");
        process::exit(1);
    }
}
