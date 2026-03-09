use rep_orchestrator::{resolver::test_plan_resolver_task, run_server, state::ServerState};
use std::{io::stdout, process};
use tokio::task::{LocalSet, spawn_local};
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

    let (state, rx) = ServerState::new();

    tokio::spawn(async move {
        info!("starting server");
        if let Err(error) = run_server(state).await {
            error!(%error, "Fatal error in axum server task");
            process::exit(1);
        }
    });

    // We need to use a local set for the non-Send futures needed to resolve test plans
    info!("spawning test plan resolver task");
    if let Err(error) = LocalSet::new()
        .run_until(async { spawn_local(test_plan_resolver_task(rx)).await })
        .await
    {
        error!(%error, "Fatal error in resolver task");
        process::exit(1);
    };
}
