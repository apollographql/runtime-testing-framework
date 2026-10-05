use crate::{
    Error,
    config::WorkloadClusters,
    db::{ClusterId, PoolId, TestExecution},
    event_loop::{Event, EventData, PendingProvision, QueueEvent},
};
use rtf_orchestrator_shared::event_queue::{
    ClusterQueueState, EventSummary, PendingProvisionSummary, PoolQueueState, SnapshotSummary,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct EventQueueInner {
    /// Pending events for in-progress executions, independent of cluster
    pending_non_provisions: VecDeque<Event>,
    /// Pending provisioning events for new executions, queued per pool
    pending_provisions: HashMap<PoolId, VecDeque<PendingProvision>>,
    /// Ordering for obtaining the next queueable provisioning event, visited round-robin
    pool_provision_order: VecDeque<PoolId>,
    /// The clusters that make up each pool
    pool_clusters: HashMap<PoolId, Vec<ClusterId>>,
    /// Pools that runs can claim dedicated clusters from
    dedicated_pools: HashSet<PoolId>,
    /// Uuids for the set of executions whose namespaces are live, per cluster
    running_executions: HashMap<ClusterId, HashSet<Uuid>>,
    /// Maximum number of live namespaces, per cluster
    max_concurrent_executions: HashMap<ClusterId, usize>,
    /// Clusters that are reserved for, or owned by, a run that requires a dedicated cluster
    claims: HashMap<ClusterId, ClusterClaim>,
}

impl EventQueueInner {
    pub(super) fn new(cfg: &WorkloadClusters) -> Self {
        Self {
            pending_non_provisions: VecDeque::new(),
            pending_provisions: HashMap::new(),
            pool_provision_order: cfg.available_pools().into(),
            pool_clusters: cfg.pool_clusters(),
            dedicated_pools: cfg.dedicated_pools(),
            running_executions: HashMap::new(),
            max_concurrent_executions: cfg.max_concurrent_executions(),
            claims: HashMap::new(),
        }
    }

    pub(super) fn running_on(&self, cluster: &ClusterId) -> usize {
        self.running_executions.get(cluster).map_or(0, |s| s.len())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pending_provisions.values().all(|q| q.is_empty())
            && self.pending_non_provisions.is_empty()
    }

    pub(super) fn push_event(&mut self, evt: QueueEvent) {
        match evt {
            QueueEvent::Provision(pool, evt) => self
                .pending_provisions
                .entry(pool)
                .or_default()
                .push_back(evt),
            QueueEvent::Other(evt) => self.pending_non_provisions.push_back(evt),
        }
    }

    pub(super) fn next_event(&mut self) -> Option<Event> {
        if let Some(evt) = self.pending_non_provisions.pop_front() {
            return Some(evt);
        } else if let Some(evt) = self.runnable_provisioning_event() {
            self.insert_running_execution(evt.test_execution.uuid(), evt.cluster.clone());
            return Some(evt);
        }

        None
    }

    /// Find the next pool in round-robin order with a queued provisioning event that can be
    /// assigned to a cluster, and remove that event from the queue.
    ///
    /// Events within a pool are considered in order, but one that is waiting on a dedicated
    /// cluster does not block the events behind it.
    pub(super) fn runnable_provisioning_event(&mut self) -> Option<Event> {
        for _ in 0..self.pool_provision_order.len() {
            let pool = self.pool_provision_order.front()?.clone();
            self.pool_provision_order.rotate_left(1);

            let n_pending = self.pending_provisions.get(&pool).map_or(0, VecDeque::len);
            for i in 0..n_pending {
                let (run_uuid, requires_dedicated) = {
                    let pending = &self.pending_provisions[&pool][i];
                    (pending.run_uuid, pending.requires_dedicated)
                };

                let (cluster, data) = match self.try_assign(&pool, run_uuid, requires_dedicated) {
                    Some(details) => details,
                    None => continue,
                };

                let provision = self.pending_provisions.get_mut(&pool)?.remove(i)?;

                return Some(Event::new(provision.test_execution, cluster, data));
            }
        }

        None
    }

    /// Attempt to find a cluster that is able to accepts a new execution from the given test run.
    pub(super) fn try_assign(
        &mut self,
        pool: &PoolId,
        run_uuid: Uuid,
        requires_dedicated: bool,
    ) -> Option<(ClusterId, EventData)> {
        use ClusterClaim::{Acquiring, Owned, Reserved};

        let clusters = self.pool_clusters.get(pool)?;
        let cluster_with_claim = |claim| {
            clusters
                .iter()
                .find(|c| self.claims.get(*c) == Some(&claim))
        };

        // If we don't need a dedicated cluster and the pool doesn't support dedicated,
        // assign to the least loaded cluster (provided we have capacity)
        if !(requires_dedicated && self.dedicated_pools.contains(pool)) {
            let cluster = clusters
                .iter()
                .filter(|c| !self.claims.contains_key(*c) && self.has_capacity(c))
                .min_by_key(|c| self.running_on(c))?;

            return Some((cluster.clone(), EventData::ResolveConfig));
        }

        // All assignments past this point are for runs requiring dedicated clusters

        // If this run already owns a cluster, assign to that cluster (provided we have capacity)
        if let Some(cluster) = cluster_with_claim(Owned(run_uuid)) {
            return self
                .has_capacity(cluster)
                .then(|| (cluster.clone(), EventData::ResolveConfig));
        }

        // If this run is in the process of acquiring its cluster then all other executions block
        // until that process has finished
        if cluster_with_claim(Acquiring(run_uuid)).is_some() {
            return None;
        }

        // If this run has an outstanding reservation assign to that cluster if it is now free and
        // update the claim to Acquiring (otherwise we continue waiting)
        if let Some(cluster) = cluster_with_claim(Reserved(run_uuid)) {
            if self.running_on(cluster) > 0 || !self.has_capacity(cluster) {
                return None;
            }
            self.claims.insert(cluster.clone(), Acquiring(run_uuid));
            return Some((cluster.clone(), EventData::AcquireCluster));
        }

        let unclaimed: Vec<&ClusterId> = clusters
            .iter()
            .filter(|c| !self.claims.contains_key(*c))
            .collect();

        // If we have an empty unclaimed cluster, claim it and assign to it
        let first_empty = unclaimed.iter().find(|c| self.running_on(c) == 0);
        if let Some(cluster) = first_empty {
            self.claims.insert((*cluster).clone(), Acquiring(run_uuid));

            return Some(((*cluster).clone(), EventData::AcquireCluster));
        }

        // Finally, if we have at least one unclaimed cluster then reserve the one with the fewest
        // ongoing executions before returning None
        if let Some(cluster) = unclaimed.iter().min_by_key(|c| self.running_on(c)) {
            self.claims.insert((*cluster).clone(), Reserved(run_uuid));
        }

        None
    }

    pub(super) fn has_capacity(&self, cluster: &ClusterId) -> bool {
        let max = self
            .max_concurrent_executions
            .get(cluster)
            .copied()
            .unwrap_or(0);

        self.running_on(cluster) < max
    }

    /// Immediately remove any outstanding reservations and schedule a ReleaseCluster event for any
    /// clusters currently claimed by this run.
    pub(super) fn schedule_cluster_release(&mut self, run_uuid: Uuid, ex: &TestExecution) {
        let mut released = Vec::new();

        self.claims.retain(|cluster, claim| match claim {
            ClusterClaim::Reserved(run) if *run == run_uuid => false,
            ClusterClaim::Acquiring(run) | ClusterClaim::Owned(run) if *run == run_uuid => {
                released.push(cluster.clone());
                true
            }
            _ => true,
        });

        for cluster in released.into_iter() {
            self.pending_non_provisions.push_front(Event::new(
                ex.clone(),
                cluster,
                EventData::ReleaseCluster,
            ));
        }
    }

    /// Remove any claims currently in place for the given cluster ID
    pub(super) fn clear_cluster_claims_for(&mut self, cluster: &ClusterId) {
        self.claims.remove(cluster);
    }

    /// Mark the `cluster` as being owned by `run_uuid`
    pub(super) fn mark_cluster_as_owned(&mut self, cluster: &ClusterId, run_uuid: Uuid) {
        self.claims
            .insert(cluster.clone(), ClusterClaim::Owned(run_uuid));
    }

    #[cfg(test)]
    pub(super) fn cluster_claim_for(&self, cluster: &ClusterId) -> Option<ClusterClaim> {
        self.claims.get(cluster).cloned()
    }

    pub(super) fn insert_running_execution(&mut self, ex_uuid: Uuid, cluster: ClusterId) {
        self.running_executions
            .entry(cluster.clone())
            .or_default()
            .insert(ex_uuid);
    }

    pub(super) fn remove_running_execution(&mut self, ex_uuid: Uuid) -> bool {
        for running in self.running_executions.values_mut() {
            if running.remove(&ex_uuid) {
                return true;
            }
        }

        false
    }

    pub(super) fn cluster_for_execution(&mut self, ex_uuid: Uuid) -> Option<ClusterId> {
        for (cid, running) in self.running_executions.iter() {
            if running.contains(&ex_uuid) {
                return Some(cid.clone());
            }
        }

        None
    }

    pub(super) fn abort_pending_provisions_for(&mut self, run_uuid: Uuid) -> Vec<TestExecution> {
        let mut aborted = Vec::new();

        for evts in self.pending_provisions.values_mut() {
            evts.retain(|evt| {
                if evt.run_uuid == run_uuid {
                    aborted.push(evt.test_execution.clone());
                    false
                } else {
                    true
                }
            });
        }

        aborted
    }

    pub(super) fn fallback_cluster_for_pool(&self, pool: PoolId) -> crate::Result<ClusterId> {
        self.pool_clusters
            .get(&pool)
            .and_then(|c| c.first().cloned())
            .ok_or_else(|| Error::UnknownWorkloadPool {
                pool: pool.to_string(),
            })
    }

    pub(super) fn iter_claims(&self) -> impl Iterator<Item = (&ClusterId, &ClusterClaim)> {
        self.claims.iter()
    }

    pub(super) fn execution_ids_for_cluster(
        &self,
        cluster: &ClusterId,
    ) -> impl Iterator<Item = &Uuid> {
        self.running_executions.get(cluster).into_iter().flatten()
    }

    pub(super) fn pending_provisions_for_pool(&self, pool: &PoolId) -> impl Iterator<Item = Uuid> {
        self.pending_provisions
            .get(pool)
            .into_iter()
            .flatten()
            .map(|evt| evt.test_execution.uuid())
    }

    pub(super) fn purge_execution_state(&mut self, ex: TestExecution) -> Option<()> {
        let ex_id = ex.uuid();

        // drop provision events first to ensure that we don't create any _new_ namespaces
        for events in self.pending_provisions.values_mut() {
            events.retain(|evt| evt.test_execution.uuid() != ex_id)
        }

        if let Some(cluster) = self.cluster_for_execution(ex_id) {
            // The execution made it as far as being provisioned so we need to nuke the namespace
            self.running_executions.get_mut(&cluster)?.remove(&ex_id);
            self.pending_non_provisions
                .retain(|evt| evt.test_execution.uuid() != ex_id);
            self.pending_non_provisions.push_front(Event::new(
                ex,
                cluster,
                EventData::PurgeNamespace,
            ));
        };

        None
    }

    pub(super) fn snapshot_state(&self) -> InnerSnapshotState {
        let mut summary = SnapshotSummary::default();
        let mut pools: BTreeMap<String, PoolQueueState> = BTreeMap::new();
        let mut clusters: BTreeMap<String, ClusterQueueState> = BTreeMap::new();

        for (pool_id, events) in self.pending_provisions.iter() {
            let state = pools.entry(pool_id.to_string()).or_default();
            for evt in events.iter() {
                state.pending_provisions.push(PendingProvisionSummary {
                    execution_id: evt.test_execution.uuid(),
                    run_id: evt.run_uuid,
                    requires_dedicated: evt.requires_dedicated,
                });
            }
        }

        for (cluster_id, running) in self.running_executions.iter() {
            let state = clusters.entry(cluster_id.to_string()).or_default();
            state.running_executions.extend(running.iter().copied());
            state.running_executions.sort_unstable();
            summary.running += running.len();
        }

        for evt in self.pending_non_provisions.iter() {
            let state = clusters.entry(evt.cluster.to_string()).or_default();
            state.pending_non_provisions.push(event_summary(evt));
            summary.pending_non_provisions += 1;
        }

        // Ensure that we have default summaries for pools/clusters that are currently empty
        for (pool_id, cluster_ids) in self.pool_clusters.iter() {
            pools.entry(pool_id.to_string()).or_default();

            for cluster_id in cluster_ids.iter() {
                clusters.entry(cluster_id.to_string()).or_default();
            }
        }

        let claims: Vec<_> = self
            .claims
            .iter()
            .map(|(cid, claim)| (cid.to_string(), *claim, self.running_on(cid)))
            .collect();

        InnerSnapshotState {
            summary,
            claims,
            pools,
            clusters,
        }
    }
}

// Helpers for tests of the rest of the event queue logic to get access at inner
// state for making assertions
#[cfg(test)]
impl EventQueueInner {
    pub(super) fn pending_non_provisions(&self) -> &VecDeque<Event> {
        &self.pending_non_provisions
    }

    pub(super) fn pending_provisions(&self) -> &HashMap<PoolId, VecDeque<PendingProvision>> {
        &self.pending_provisions
    }

    pub(super) fn running_executions(&self) -> &HashMap<ClusterId, HashSet<Uuid>> {
        &self.running_executions
    }

    pub(super) fn claims_mut(&mut self) -> &mut HashMap<ClusterId, ClusterClaim> {
        &mut self.claims
    }
}

fn event_summary(evt: &Event) -> EventSummary {
    EventSummary {
        execution_id: evt.test_execution.uuid(),
        event: evt.data.name().to_string(),
    }
}

pub(super) struct InnerSnapshotState {
    pub(super) summary: SnapshotSummary,
    pub(super) claims: Vec<(String, ClusterClaim, usize)>,
    pub(super) pools: BTreeMap<String, PoolQueueState>,
    pub(super) clusters: BTreeMap<String, ClusterQueueState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClusterClaim {
    /// The run is waiting for the executions currently running on the cluster to finish
    Reserved(Uuid),
    /// The run has the cluster to itself and is in the process of running acquisition actions
    Acquiring(Uuid),
    /// The run has the cluster to itself and it is ready for its executions to start
    Owned(Uuid),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_POOL;

    fn cluster_a() -> ClusterId {
        ClusterId::new("a")
    }

    fn cluster_b() -> ClusterId {
        ClusterId::new("b")
    }

    fn two_cluster_pool(max_concurrent: usize, dedicated: bool) -> WorkloadClusters {
        let mut clusters =
            WorkloadClusters::for_test_with_available_clusters(max_concurrent, "a", &["a", "b"]);
        clusters.cluster_pools.default.available_clusters = vec!["a".into(), "b".into()];
        clusters.cluster_pools.default.supports_dedicated = dedicated;
        clusters.cluster_pools.additional.clear();

        clusters
    }

    // Helpers for testing the try_assign method without having to run the full next_event logic
    impl EventQueueInner {
        fn try_assign_insert(&mut self) -> Option<(ClusterId, EventData)> {
            let (cluster, data) = self.try_assign(&DEFAULT_POOL, Uuid::new_v4(), false)?;
            self.insert_running_execution(Uuid::new_v4(), cluster.clone());

            Some((cluster, data))
        }

        fn try_assign_insert_dedicated(
            &mut self,
            run_uuid: Uuid,
        ) -> Option<(ClusterId, EventData)> {
            let (cluster, data) = self.try_assign(&DEFAULT_POOL, run_uuid, true)?;
            self.insert_running_execution(Uuid::new_v4(), cluster.clone());

            Some((cluster, data))
        }
    }

    fn acquire(cluster: &str) -> (ClusterId, EventData) {
        (ClusterId::new(cluster), EventData::AcquireCluster)
    }

    fn resolve(cluster: &str) -> (ClusterId, EventData) {
        (ClusterId::new(cluster), EventData::ResolveConfig)
    }

    #[tokio::test]
    async fn try_assign_balances_executions_across_the_clusters_in_a_pool() {
        let mut inner = EventQueueInner::new(&two_cluster_pool(10, false));

        assert_eq!(inner.try_assign_insert(), Some(resolve("a")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("a")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
    }

    #[tokio::test]
    async fn try_assign_only_uses_clusters_with_capacity() {
        let mut inner = EventQueueInner::new(&two_cluster_pool(1, false));
        inner.insert_running_execution(Uuid::new_v4(), cluster_a());

        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
        assert_eq!(inner.try_assign_insert(), None);
    }

    #[tokio::test]
    async fn try_assign_assigns_a_dedicated_run_to_a_single_cluster() {
        let mut inner = EventQueueInner::new(&two_cluster_pool(2, true));
        let run = Uuid::new_v4();

        assert_eq!(inner.try_assign_insert_dedicated(run), Some(acquire("a")));
        assert_eq!(
            inner.claims.get(&cluster_a()),
            Some(&ClusterClaim::Acquiring(run))
        );

        // No further executions for the run until the cluster has been acquired
        assert_eq!(inner.try_assign_insert_dedicated(run), None);

        // Completing acquisition should then allow further executions to resolve up to the
        // cluster's capacity
        inner.mark_cluster_as_owned(&cluster_a(), run);
        assert_eq!(inner.try_assign_insert_dedicated(run), Some(resolve("a")));
        assert_eq!(inner.try_assign_insert_dedicated(run), None, "no capacity");
    }

    #[tokio::test]
    async fn try_assign_respects_dedicated_clusters() {
        let mut inner = EventQueueInner::new(&two_cluster_pool(10, true));
        let run = Uuid::new_v4();

        assert_eq!(inner.try_assign_insert_dedicated(run), Some(acquire("a")));
        assert_eq!(inner.try_assign_insert(), Some(resolve("b")));
        assert_eq!(inner.try_assign_insert_dedicated(run), None);

        inner.claims.insert(cluster_a(), ClusterClaim::Owned(run));
        assert_eq!(inner.try_assign_insert_dedicated(run), Some(resolve("a")));
    }

    #[tokio::test]
    async fn try_assign_respects_reservations() {
        let mut inner = EventQueueInner::new(&two_cluster_pool(10, true));
        let ex = Uuid::new_v4();
        let run = Uuid::new_v4();

        inner.insert_running_execution(ex, cluster_a());
        inner.insert_running_execution(Uuid::new_v4(), cluster_b());
        inner.insert_running_execution(Uuid::new_v4(), cluster_b());

        // we should reserve the cluster with the fewest running executions
        assert_eq!(inner.try_assign_insert_dedicated(run), None);
        assert_eq!(
            inner.claims.get(&cluster_a()),
            Some(&ClusterClaim::Reserved(run))
        );

        // once the reserved cluster has drained we should be able to acquire
        inner.remove_running_execution(ex);
        assert_eq!(inner.try_assign_insert_dedicated(run), Some(acquire("a")));
    }

    #[tokio::test]
    async fn try_assign_reserves_multiple_clusters() {
        let mut inner = EventQueueInner::new(&two_cluster_pool(10, true));

        inner.insert_running_execution(Uuid::new_v4(), cluster_a());
        inner.insert_running_execution(Uuid::new_v4(), cluster_b());

        for (run, cluster) in [(Uuid::new_v4(), cluster_a()), (Uuid::new_v4(), cluster_b())] {
            assert_eq!(inner.try_assign_insert_dedicated(run), None);
            assert_eq!(
                inner.claims.get(&cluster),
                Some(&ClusterClaim::Reserved(run))
            );
        }
    }
}
