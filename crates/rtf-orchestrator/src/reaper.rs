use crate::{
    config::Config,
    event_loop::EventQueueState,
    k8s::{self, ClusterClients, WorkloadClient},
};
use std::time::Duration;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{debug, error, info};
use uuid::Uuid;

pub async fn stale_namespace_reaper_task(eq_state: EventQueueState, cfg: &'static Config) {
    let mut ticker = interval(Duration::from_secs(
        cfg.workload_clusters.orphaned_namespace_prune_interval_secs,
    ));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;

        for cluster_cfg in cfg.workload_clusters.available_clusters.iter() {
            let cluster = &cluster_cfg.name;
            let mut client = match ClusterClients::try_new_workload(
                &cluster_cfg.kubeconfig_path(),
                &cluster_cfg.workload_context,
            )
            .await
            {
                Ok(client) => client,
                Err(error) => {
                    error!(%cluster, %error, "failed to construct k8s client for namespace reaper task");
                    continue;
                }
            };

            if let Err(error) =
                delete_orphaned_cluster_namespaces(cluster, &eq_state, &mut client).await
            {
                error!(%cluster, %error, "failed to delete orphaned namespaces");
            }
        }
    }
}

async fn delete_orphaned_cluster_namespaces<C>(
    cluster: &str,
    eq_state: &EventQueueState,
    client: &mut C,
) -> k8s::Result<()>
where
    C: WorkloadClient,
{
    // We list namespaces before checking the current in-flight set to ensure that newly created
    // namespaces are not accidentally removed. If we do this the other way round (or only check
    // in-flight executions above in the reaper task) then we run the risk of accidentally deleting
    // more than we should.
    let namespaces = client.list_workload_namespaces().await?;
    let in_flight = eq_state.in_flight_executions().await;

    // Currently the condition for a namespace to be considered orphaned is:
    // - its name parses as a UUID
    // - that UUID does not belong to an in-flight execution
    //
    // This prevents us attempting to delete other namespaces within the cluster that we didn't
    // create, but will be an incomplete set if we ever move to supporting multiple namespaces
    // per-execution or alter our naming conventions for execution namespaces).
    let orphaned_namespaces = namespaces
        .into_iter()
        .filter(|ns| matches!(ns.parse::<Uuid>(), Ok(id) if !in_flight.contains(&id)));

    for ns in orphaned_namespaces {
        info!(%cluster, %ns, "deleting orphaned namespace");
        match client.delete_workload_namespace(&ns).await {
            Ok(_) => (),

            Err(k8s::Error::Kube(kube::Error::Api(ref e))) if e.code == 404 => {
                debug!(%cluster, %ns, "orphaned namespace already gone");
            }

            Err(error) => error!(%cluster, %ns, %error, "failed to delete orphaned namespace"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        event_loop::EventQueue,
        k8s::mock_client::{MockClient, Resp},
    };
    use simple_test_case::test_case;

    async fn eq_state_with(in_flight: &[Uuid]) -> EventQueueState {
        let cfg = Config::for_test();
        let (_, prov_handle, eq_state, _) = EventQueue::new(&cfg.workload_clusters);

        for id in in_flight.iter() {
            prov_handle
                .register_existing_execution(*id, Uuid::new_v4())
                .await;
        }

        eq_state
    }

    fn u(v: u128) -> Uuid {
        Uuid::from_u128(v)
    }

    #[test_case(&[], &[], &[]; "nothing running")]
    #[test_case(&[u(0), u(1)], &[], &[]; "only live")]
    #[test_case(&[u(0)], &[u(1)], &[u(1)]; "live and orphaned")]
    #[test_case(&[], &[u(0)], &[u(0)]; "only orphaned")]
    #[tokio::test]
    async fn delete_orphaned_cluster_namespaces_deletes_correct_namespaces(
        live: &[Uuid],
        orphaned: &[Uuid],
        expected_deletes: &[Uuid],
    ) {
        let eq_state = eq_state_with(live).await;
        let mut client = MockClient {
            list_workload_namespaces: Resp::new(Ok(live
                .iter()
                .chain(orphaned.iter())
                .map(|u| u.to_string())
                // Including non-execution namespaces to ensure that our filtering is working and
                // that we don't attempt to delete them
                .chain(["kube-system".to_string(), "default".to_string()])
                .collect())),
            delete_workload_namespace: Resp::new(Ok(())),
            ..Default::default()
        };

        let res = delete_orphaned_cluster_namespaces("alpha", &eq_state, &mut client).await;
        assert!(res.is_ok(), "{res:?}");

        let deleted = client.deleted_namespaces.lock().unwrap().clone();
        let expected: Vec<String> = expected_deletes.iter().map(|u| u.to_string()).collect();

        assert_eq!(deleted, expected);
    }
}
