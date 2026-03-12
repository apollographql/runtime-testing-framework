use rep_orchestrator_poc::run_server;
use std::{io::stdout, process};
use tracing::{error, info, subscriber::set_global_default};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(stdout)
        .json()
        .flatten_event(true)
        .with_span_list(true)
        .with_current_span(false)
        // .with_file(true)
        // .with_line_number(true)
        // .with_thread_ids(true)
        .finish();

    // TODO: wire up a reload handle for runtime setting of the logging filter

    set_global_default(subscriber).expect("unable to set a global tracing subscriber");

    info!("starting server");
    if let Err(error) = run_server().await {
        error!(%error, "Fatal error in axum server task");
        process::exit(1);
    }
}
