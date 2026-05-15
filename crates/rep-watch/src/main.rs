use rep_orchestrator_shared::summary::TestRunSummary;
use rep_watch::runner::Runner;
use std::time::{Duration, Instant};
use tokio::time::sleep;

const POLL_SECONDS: u64 = 5;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut runner = Runner::try_new().await?;
    let mut footer = String::new();

    let (tp_path, mut trs) = runner.get_initial_summary().await?;
    let handle = runner.spawn_event_filter().await?;
    runner.render_run_details(&mut trs, &tp_path, &footer).await;

    while any_execution_ongoing(&trs) {
        match runner.get_run_status(trs.id).await {
            Ok(new) => trs = new,
            Err(e) => footer = format!("failed to pull run update: {e}"),
        }

        runner.render_run_details(&mut trs, &tp_path, &footer).await;
        footer.clear();
        sleep(Duration::from_secs(POLL_SECONDS)).await;
    }

    let t = Instant::now().duration_since(runner.start).as_secs();

    runner
        .append_buffer_content(format!("\nRun completed in {t}s"))
        .await;

    // wait for the +rtf buffer to close so we still run the event filter
    _ = handle.await;

    println!("exiting");

    Ok(())
}

fn any_execution_ongoing(trs: &TestRunSummary) -> bool {
    !trs.current_status.is_terminal()
        || trs
            .executions
            .iter()
            .any(|ex| !ex.current_status.is_terminal())
}
