use crate::{
    Result, conn,
    db::{Status, StatusTracked},
    state::TestRunWithPayload,
};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info_span, warn};

pub async fn resolver_task(mut rx: UnboundedReceiver<TestRunWithPayload>) -> Result<()> {
    let conn = conn!();

    while let Some(TestRunWithPayload { test_run, payload }) = rx.recv().await {
        let span = info_span!("resolve", test_run_id = %test_run.uuid(), name = %test_run.name());
        let _guard = span.enter();

        // TODO: The logic here is temporary. We need to ensure that we get status updates for the
        // test run and that the executions are created in order to be able to test the status
        // endpoints.

        if let Err(e) = test_run.set_status(Status::Resolving, None, conn).await {
            error!("Unable to set test run status to Resolving: {e}");
            continue;
        }

        let it = match payload.test_plan.try_iter_matrix_variants() {
            Ok(it) => it,
            Err(error) => {
                warn!(%error, "unable to expand matrix variants");
                if let Err(e) = test_run.set_status(Status::Unrunnable, None, conn).await {
                    error!("Unable to set test run status to Unrunnable: {e}");
                }
                continue;
            }
        };

        for (name, _variant) in it {
            let ex = match test_run.init_execution(&name, conn).await {
                Ok(ex) => ex,
                Err(_) => continue,
            };
            _ = ex.set_status(Status::Resolving, None, conn).await;
        }
    }

    warn!("Test plan resolver channel closed. Exiting resolver task");

    Ok(())
}
