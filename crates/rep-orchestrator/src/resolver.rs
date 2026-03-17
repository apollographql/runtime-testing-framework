use crate::{
    Result, conn,
    db::{Status, StatusTracked},
    state::TestRunWithPayload,
};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info, info_span, warn};

pub async fn resolver_task(mut rx: UnboundedReceiver<TestRunWithPayload>) -> Result<()> {
    let conn = conn!();

    while let Some(TestRunWithPayload { test_run, .. }) = rx.recv().await {
        let span = info_span!("resolve", test_run_id = %test_run.uuid(), name = %test_run.name());
        let _guard = span.enter();

        match test_run.set_status(Status::Resolving, None, conn).await {
            Ok(_) => {
                info!("Test run status set to Resolving");
                continue;
            }
            Err(e) => {
                error!("Unable to set test run status to Resolving: {e}");
                continue;
            }
        }
    }

    warn!("Test plan resolver channel closed. Exiting resolver task");

    Ok(())
}
