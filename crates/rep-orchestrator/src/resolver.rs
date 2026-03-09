//! Long lived task for resolving test plans
use rtf_config::formats::TestPlanConfig;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{debug, info, info_span, warn};
use uuid::Uuid;

#[derive(Debug)]
pub struct TestPlanWithId {
    pub id: Uuid,
    pub test_plan: TestPlanConfig,
}

pub async fn test_plan_resolver_task(mut rx: UnboundedReceiver<TestPlanWithId>) {
    loop {
        let TestPlanWithId { id, test_plan } = match rx.recv().await {
            Some(tp) => tp,
            None => {
                info!("Test plan resolver channel closed. Exiting resolver task");
                return;
            }
        };

        let span = info_span!("resolve_test_plan", %id, test_plan_name=%test_plan.name);
        let _guard = span.enter();
        info!("expanding test plan variants");

        let it = match test_plan.try_iter_matrix_variants() {
            Ok(it) => it,
            Err(error) => {
                warn!(%error, "unable to expand matrix variants");
                continue;
            }
        };

        for (name, _variant) in it {
            debug!(%name, "got variant");
        }
    }
}
