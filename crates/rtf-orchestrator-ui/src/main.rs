use rtf_orchestrator_ui::run_server;
use std::process;
use tracing::error;
use tracing_subscriber::EnvFilter;

// glibc malloc (prod runtime is debian/glibc) does not return freed memory to the
// OS after large transient allocation spikes, so RSS sticks at peak. jemalloc with
// background purging returns dirty pages on a decay timer.
#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(std::io::stdout)
        .init();

    // reqwest's rustls backend needs a process-wide crypto provider installed before the first
    // client is built. `install_default` only errors if one is already set, which is harmless.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    if let Err(error) = run_server().await {
        error!(%error, "fatal error");
        process::exit(1);
    }
}
