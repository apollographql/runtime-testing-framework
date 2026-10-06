use crate::{
    config::WorkloadClusterConfig,
    db::{ClusterId, TestExecution, UpdateHandle},
    event_loop::{EventData, EventQueue, Result},
    k8s::{ClusterClients, WorkloadClient},
};
use tracing::{error, info};

impl EventQueue {
    pub(super) async fn handle_acquire<H>(
        &mut self,
        ex: &TestExecution,
        cluster: &ClusterId,
        cluster_cfg: &WorkloadClusterConfig,
        conn: &mut H,
    ) -> Result<Option<EventData>>
    where
        H: UpdateHandle,
    {
        let mut clients = ClusterClients::try_new_workload(
            &cluster_cfg.kubeconfig_path(),
            &cluster_cfg.workload_context,
        )
        .await?;

        self.handle_acquire_inner(ex, cluster, &mut clients, conn)
            .await
    }

    async fn handle_acquire_inner<K, H>(
        &mut self,
        ex: &TestExecution,
        cluster: &ClusterId,
        clients: &mut K,
        conn: &mut H,
    ) -> Result<Option<EventData>>
    where
        K: WorkloadClient,
        H: UpdateHandle,
    {
        let ex_id = ex.uuid();

        let (run_uuid, config) = self
            .with_shared(|shared| shared.run_and_workload_config(ex_id))
            .await?;

        if config.node_label_weights.is_empty() {
            self.with_inner(|inner| inner.mark_cluster_as_owned(cluster, run_uuid))
                .await;

            return Ok(Some(EventData::ResolveConfig));
        }

        info!(%ex_id, %cluster, "labelling nodes");
        let res = async {
            clients.remove_all_prefixed_labels().await?;
            clients
                .apply_node_label_allocation(config.node_label_weights)
                .await
        }
        .await;

        match res {
            Ok(_) => {
                self.with_inner(|inner| inner.mark_cluster_as_owned(cluster, run_uuid))
                    .await;

                Ok(Some(EventData::ResolveConfig))
            }
            Err(e) => {
                error!(%run_uuid, %cluster, %e, "unable to label nodes, aborting run");
                let msg = format!("unable to label dedicated cluster nodes for {cluster}: {e}");
                self.abort_run_acquiring_cluster(run_uuid, ex, msg, conn)
                    .await;

                Err(e.into())
            }
        }
    }

    pub(super) async fn handle_release(
        &mut self,
        cluster: &ClusterId,
        cluster_cfg: &WorkloadClusterConfig,
    ) -> Result<()> {
        info!(%cluster, "removing node labels");
        let res = async {
            let mut clients = ClusterClients::try_new_workload(
                &cluster_cfg.kubeconfig_path(),
                &cluster_cfg.workload_context,
            )
            .await?;

            clients.remove_all_prefixed_labels().await
        }
        .await;

        self.with_inner(|inner| inner.clear_cluster_claims_for(cluster))
            .await;

        Ok(res?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        context::OrchestratorContext,
        db::{MockUpdateHandle, Status, StatusUpdate, TaggedStatusUpdate},
        event_loop::{self, inner::ClusterClaim, tests::stub_test_plan},
        k8s::{
            self, NodeAllocationError,
            mock_client::{MockClient, Resp},
        },
        resolver::ResolverError,
    };
    use rtf_orchestrator_shared::{payload::SourceKeyedArrayMap, workload_config::WorkloadConfig};
    use simple_test_case::test_case;
    use std::assert_matches;
    use uuid::Uuid;

    fn alpha_cluster() -> ClusterId {
        ClusterId::new("alpha")
    }

    fn empty_source_map<T>() -> SourceKeyedArrayMap<T> {
        SourceKeyedArrayMap {
            keys: vec![],
            data: vec![],
        }
    }

    fn labelling_cfg() -> WorkloadConfig {
        WorkloadConfig {
            allow_k8s_write: false,
            node_label_weights: vec![("a".into(), 1), ("b".into(), 1)],
        }
    }

    #[tokio::test]
    async fn handle_acquire_errors_on_unknown_execution() {
        let cfg = Config::for_test();
        let (mut q, _, _, _) = EventQueue::new(&cfg.workload_clusters);
        let mut clients = MockClient::default_ok();
        let mut handle = MockUpdateHandle::default();
        let ex = TestExecution::create_stub(0, 0, 0, "no parent run");

        let res = q
            .handle_acquire_inner(&ex, &alpha_cluster(), &mut clients, &mut handle)
            .await;

        assert_matches!(
            res,
            Err(event_loop::Error::Resolve(ResolverError::UnknownExecution(
                uuid
            ))) if uuid == ex.uuid()
        );
    }

    async fn setup_acquire_test(wcfg: WorkloadConfig) -> (TestExecution, Uuid, EventQueue) {
        let cfg = Config::for_test();
        let (q, ph, _, _) = EventQueue::new(&cfg.workload_clusters);

        let ctx = OrchestratorContext::new_from_inlined_files(
            &Config::for_test(),
            empty_source_map(),
            empty_source_map(),
            Default::default(),
        );
        let run_uuid = Uuid::new_v4();
        ph.cache_for_test_run(
            run_uuid,
            Some("alice".to_string()),
            wcfg,
            false,
            ctx,
            stub_test_plan(),
        )
        .await;

        let ex = TestExecution::create_stub(0, 0, 0, "test");
        q.with_shared(|shared| shared.register_execution(ex.uuid(), run_uuid))
            .await;
        q.with_inner(|inner| inner.insert_running_execution(ex.uuid(), alpha_cluster()))
            .await;

        (ex, run_uuid, q)
    }

    #[test_case(WorkloadConfig::default(), false; "without label weights")]
    #[test_case(labelling_cfg(), true; "with label weights")]
    #[tokio::test]
    async fn handle_acquire_removes_and_applies_labels(
        wcfg: WorkloadConfig,
        should_set_labels: bool,
    ) {
        let (ex, run_uuid, mut q) = setup_acquire_test(wcfg).await;
        let mut handle = MockUpdateHandle::default();
        let mut clients = MockClient::default_ok();

        // The asserts below for rely on these initially being set
        assert!(clients.remove_all_prefixed_labels.is_some());
        assert!(clients.apply_node_label_allocation.is_some());

        let res = q
            .handle_acquire_inner(&ex, &alpha_cluster(), &mut clients, &mut handle)
            .await;

        assert_matches!(res, Ok(Some(EventData::ResolveConfig)));
        assert_eq!(
            clients.remove_all_prefixed_labels.is_none(),
            should_set_labels
        );
        assert_eq!(
            clients.apply_node_label_allocation.is_none(),
            should_set_labels
        );

        let claim = q
            .with_inner(|inner| inner.cluster_claim_for(&alpha_cluster()))
            .await;
        assert_eq!(claim, Some(ClusterClaim::Owned(run_uuid)));
    }

    #[test_case(
        MockClient {
            remove_all_prefixed_labels: Resp::new(Err(k8s::Error::NodeAllocation(
                NodeAllocationError::ZeroWeights,
            ))),
            ..MockClient::default_ok()
        };
        "remove labels fails"
    )]
    #[test_case(
        MockClient {
            apply_node_label_allocation: Resp::new(Err(k8s::Error::NodeAllocation(
                NodeAllocationError::ZeroWeights,
            ))),
            ..MockClient::default_ok()
        };
        "apply labels fails"
    )]
    #[tokio::test]
    async fn handle_acquire_aborts_the_run_if_labelling_fails(mut clients: MockClient) {
        let (ex, _run_uuid, mut q) = setup_acquire_test(labelling_cfg()).await;
        let mut handle = MockUpdateHandle::with_execution(ex.clone());

        let res = q
            .handle_acquire_inner(&ex, &alpha_cluster(), &mut clients, &mut handle)
            .await;

        assert!(res.is_err());
        assert_matches!(handle.status_updates[0],
            TaggedStatusUpdate::Execution(0, StatusUpdate { status, .. })
                if status == Status::Unrunnable
        );
    }
}
