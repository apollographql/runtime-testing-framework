use crate::{
    db::{TestExecution, TestRun},
    event_loop::{Event, EventData},
    state::TestRunWithPayload,
};
use rep_orchestrator_shared::{payload::TriggerPayload, test_plan::RepTestPlan};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};
use tokio::sync::{
    Mutex, Notify,
    mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel},
};
use tracing::{error, info, warn};
use uuid::Uuid;

/// Coordinates queuing of k8s events to provide back pressure and prioritise running executions
/// over newly submitted ones.
///
/// Held by the event loop task with paired [ProvisioningHandle] and [EventQueueState] structs that
/// are used elsewhere in the codebase to submit events to the queue and introspect the current
/// queue state.
#[derive(Debug)]
pub struct EventQueue {
    /// Sender for submitting events back to the queue
    tx: UnboundedSender<Event>,
    /// Receiver for accepting new events
    rx: UnboundedReceiver<Event>,
    /// Shared state between the queue and paired structs
    shared: Arc<Mutex<Shared>>,
    /// Pending events for in-progress executions
    pending_non_provisions: VecDeque<Event>,
    /// Pending provisioning events for new executions
    pending_provisions: VecDeque<Event>,
    /// Notifier for waking up the resolver when it waits for an available namespace slot
    notify: Arc<Notify>,
}

impl EventQueue {
    /// Construct a new [EventQueue] along with its paired [ProvisioningHandle] and [EventQueueState]
    /// structs.
    pub fn new(
        max_concurrent_executions: usize,
        max_queued_executions: usize,
    ) -> (
        Self,
        ProvisioningHandle,
        EventQueueState,
        UnboundedReceiver<TestRunWithPayload>,
    ) {
        let shared = Arc::new(Mutex::new(Shared {
            max_concurrent_executions,
            max_queued_executions,
            running_executions: HashSet::new(),
            n_queued: 0,
        }));
        let notify = Arc::new(Notify::new());
        let (tx_resolve, rx_resolve) = unbounded_channel();
        let (tx, rx) = unbounded_channel();

        let eq = EventQueue {
            tx,
            rx,
            shared,
            pending_provisions: VecDeque::new(),
            pending_non_provisions: VecDeque::new(),
            notify,
        };

        let ph = ProvisioningHandle {
            shared: eq.shared.clone(),
            tx: eq.tx.clone(),
            notify: eq.notify.clone(),
        };

        let eqs = EventQueueState {
            shared: eq.shared.clone(),
            tx_resolve,
        };

        (eq, ph, eqs, rx_resolve)
    }

    pub fn tx(&self) -> UnboundedSender<Event> {
        self.tx.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
            && self.pending_provisions.is_empty()
            && self.pending_non_provisions.is_empty()
    }

    #[inline(always)]
    fn push_event(&mut self, evt: Event) {
        match &evt.data {
            EventData::CreateEnvConfigMap(_, _) => self.pending_provisions.push_back(evt),
            _ => self.pending_non_provisions.push_back(evt),
        }
    }

    /// Returns the next [Event] to be processed, prioritising non-provision events over
    /// provisioning new namespaces.
    ///
    /// We buffer events internally and fully drain the channel of any events received since the
    /// last call to `next_event`. This method only blocks when there are no internally buffered
    /// events and the channel is currently empty.
    ///
    /// Returns [None] when the event channel is closed as per [UnboundedReceiver::recv] or if the
    /// only remaining `sender` is the one held within this struct.
    pub async fn next_event(&mut self) -> Option<Event> {
        // If our channel is empty and we have nothing queued we block and wait for the next event
        // to arrive.
        if self.is_empty() {
            let evt = self.rx.recv().await?;
            self.push_event(evt);
        }

        // Drain any pending events from the channel before we determine which event to handle next
        loop {
            // If only our own sender remains, all external senders have been dropped and
            // no new events will arrive from outside the event loop.
            if self.rx.sender_strong_count() <= 1 {
                return None;
            }

            match self.rx.try_recv() {
                Ok(evt) => self.push_event(evt),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => unreachable!("we hold a sender"),
            }
        }

        assert!(
            !(self.pending_non_provisions.is_empty() && self.pending_provisions.is_empty()),
            "should have at least one event at this point"
        );

        // Prefer progressing ongoing executions over provisioning new ones
        self.pending_non_provisions
            .pop_front()
            .or_else(|| self.pending_provisions.pop_front())
    }

    /// Remove the given execution from the running set and notify the resolver task that a
    /// namespace slot is now available.
    pub async fn mark_execution_complete(&self, execution_id: Uuid) {
        // Ensure that we release the mutex before notifying the resolver task.
        {
            let mut shared = self.shared.lock().await;
            if !shared.running_executions.remove(&execution_id) {
                warn!(%execution_id, "mark_execution_complete called for unknown execution id");
            }
        }

        // Always notify: the namespace is gone regardless of whether we tracked it, and the
        // resolver must be woken so it can re-check capacity.
        self.notify.notify_one();
    }
}

/// A handle for submitting provisioning requests to the [EventQueue].
#[derive(Debug, Clone)]
pub struct ProvisioningHandle {
    /// Shared state with the parent event queue
    shared: Arc<Mutex<Shared>>,
    /// Sender for submitting provisioning events to the event queue
    tx: UnboundedSender<Event>,
    /// Notifier used to wait for namespace capacity
    notify: Arc<Notify>,
}

impl ProvisioningHandle {
    /// Wait for a namespace slot to become available.
    ///
    /// Assumes that the caller is the only task waiting for namespace capacity.
    async fn wait_for_namespace_capacity(&self) {
        // We need to make sure that we release the mutex once we've checked capacity to avoid
        // deadlock.
        let at_or_over_capacity = {
            let shared = self.shared.lock().await;
            let current = shared.running_executions.len();
            let max = shared.max_concurrent_executions;

            if current > max {
                error!(
                    "currently have {current} active namespaces but should be capped at a max of {max}"
                );
            }

            // Using `>=` to ensure that we always wait even if we somehow manage to provision more
            // executions than we should have.
            current >= max
        };

        if at_or_over_capacity {
            info!("waiting for an available namespace slot");
            self.notify.notified().await
        }
    }

    /// Atomically decrement n_queued, add `ex` to the running executions set and send the
    /// provisioning event to the event loop.
    ///
    /// Returns `false` if unable to send the event to the event loop, otherwise `true`.
    pub async fn request_provisioning(&self, ex: TestExecution, tp: RepTestPlan) -> bool {
        // We don't need to worry about a race condition between waiting for capacity and
        // re-acquiring the lock as we are the only task attempting to provision executions.
        self.wait_for_namespace_capacity().await;

        // Ensure that we release the mutex before sending the event
        {
            let mut shared = self.shared.lock().await;
            assert!(
                shared.n_queued > 0,
                "request_provisioning called with n_queued == 0"
            );

            shared.n_queued -= 1;
            shared.running_executions.insert(ex.uuid());
        }

        if let Err(e) = self.tx.send(Event {
            test_execution: ex,
            data: EventData::CreateEnvConfigMap(tp.environment.execution, tp.scenario.execution),
        }) {
            // If the channel is closed then we're shutting down so dropping the event details here
            // is intentional.
            error!(%e, "event loop channel closed");
            return false;
        }

        true
    }

    /// Return `n` queued execution claims back to the shared state.
    ///
    /// # Safety
    /// The caller must pass an `n` that matches the number of unused claims they previously
    /// obtained from `try_reserve_pending_executions`.
    pub async unsafe fn release_pending_execution_claim(&self, n: usize) {
        let mut shared = self.shared.lock().await;
        shared.n_queued -= n;
    }
}

/// Errors that can be encountered when attempting to submit a test plan to the resolver task.
#[derive(Debug)]
pub enum SubmitError {
    /// The size of the provided claim did not match the number of test plan executions submitted
    InvalidClaim,
    /// Resolver channel is closed
    ResolveChannelClosed,
}

/// A claim for resolving a specific number of test executions.
///
/// This type is deliberately opaque so the owner is only able to pass it back to
/// [EventQueueState::try_submit_test_plan].
pub struct Claim(usize);

/// Access to shared [EventQueue] state.
///
/// Used to atomically reserve space for queuing new test executions in the resolver task.
#[derive(Debug, Clone)]
pub struct EventQueueState {
    shared: Arc<Mutex<Shared>>,
    tx_resolve: UnboundedSender<TestRunWithPayload>,
}

impl EventQueueState {
    /// Attempt to submit a [TestRunWithPayload] through to the resolver task if we are able to
    /// obtain sufficient pending execution claims.
    pub async fn try_submit_test_plan(
        &self,
        claim: Claim,
        test_run: TestRun,
        payload: TriggerPayload,
    ) -> Result<(), SubmitError> {
        let n = payload.test_plan.matrix.n_variants();
        if claim.0 != n {
            return Err(SubmitError::InvalidClaim);
        }

        match self
            .tx_resolve
            .send(TestRunWithPayload { test_run, payload })
        {
            Ok(_) => Ok(()),
            Err(_) => {
                // If we hit this branch then the channel is closed and we are likely shutting
                // down. But, we still attempt to be good citizens and release our claim on the
                // resolver queue to ensure that the shared state is correct.
                let mut shared = self.shared.lock().await;
                shared.n_queued -= n;

                Err(SubmitError::ResolveChannelClosed)
            }
        }
    }

    /// Attempt to reserve the requested number of executions if there is capacity.
    ///
    /// Returns `true` if the claim was successful, otherwise `false`.
    pub async fn try_reserve_pending_executions(&self, tp: &RepTestPlan) -> Option<Claim> {
        let n = tp.matrix.n_variants();
        let mut shared = self.shared.lock().await;

        if shared.n_queued.saturating_add(n) <= shared.max_queued_executions {
            shared.n_queued += n;
            Some(Claim(n))
        } else {
            None
        }
    }

    /// Return `n` queued execution claims back to the shared state.
    pub async fn release_pending_execution_claim(&self, claim: Claim) {
        let mut shared = self.shared.lock().await;
        shared.n_queued -= claim.0;
    }
}

#[derive(Debug, Clone)]
struct Shared {
    max_concurrent_executions: usize,
    max_queued_executions: usize,
    running_executions: HashSet<Uuid>,
    n_queued: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Queryable,
        event_loop::tests::{stub_environment, stub_scenario, stub_test_plan},
    };
    use rtf_config::templating::Scalar;
    use simple_test_case::test_case;
    use std::{collections::HashMap, time::Duration};

    #[test_case(&[(1, true)]; "single claim below max")]
    #[test_case(&[(5, true)]; "single claim at max")]
    #[test_case(&[(9, false)]; "single claim above max")]
    #[test_case(&[(5, true), (1, false)]; "second claim after max")]
    #[test_case(&[(3, true), (3, false)]; "second claim would exceed max")]
    #[test_case(&[(3, true), (2, true), (1, false)]; "seq to max then over")]
    #[tokio::test]
    async fn try_reserve_pending_executions_returns_expected_value(claims: &[(usize, bool)]) {
        let (_, _, state, _) = EventQueue::new(1, 5);

        for (i, &(n, expected)) in claims.iter().enumerate() {
            let mut tp = stub_test_plan();
            tp.matrix.dimensions = HashMap::from([("a".to_string(), vec![Scalar::Bool(true); n])]);

            let successful = state.try_reserve_pending_executions(&tp).await;
            assert_eq!(successful.is_some(), expected, "claim {i}");
        }
    }

    #[tokio::test]
    async fn mark_execution_complete_updates_running_set() {
        let (eq, _, _, _) = EventQueue::new(1, 5);
        let id = Uuid::new_v4();
        eq.shared.lock().await.running_executions.insert(id);

        eq.mark_execution_complete(id).await;

        assert!(
            !eq.shared.lock().await.running_executions.contains(&id),
            "execution was still present in running set"
        );
    }

    #[test_case(true; "known")]
    #[test_case(false; "unknown")]
    #[tokio::test]
    async fn mark_execution_complete_unblocks_waiting_provisioning_handle(
        is_known_execution: bool,
    ) {
        let (q, h, _, _) = EventQueue::new(1, 5);
        let id = Uuid::new_v4();

        // We warn if the execution was unknown but we should always notify regardless
        if is_known_execution {
            q.shared.lock().await.running_executions.insert(id);
        }

        let wait_task = tokio::spawn(async move {
            h.wait_for_namespace_capacity().await;
        });

        q.mark_execution_complete(id).await;

        tokio::time::timeout(Duration::from_secs(1), wait_task)
            .await
            .expect("provisioning handle should have been unblocked within 1s")
            .expect("wait task should not have panicked");
    }

    #[tokio::test]
    async fn request_provisioning_happy_path() {
        let (mut q, h, _, _) = EventQueue::new(1, 5);
        let ex = TestExecution::create_stub(1, 1, "test");
        let ex_uuid = ex.uuid();
        q.shared.lock().await.n_queued = 1;

        let successful = h.request_provisioning(ex, stub_test_plan()).await;

        assert!(successful, "should have been able to send event");

        let shared = q.shared.lock().await;
        assert_eq!(shared.n_queued, 0, "n_queued should have been decremented");
        assert!(
            shared.running_executions.contains(&ex_uuid),
            "execution should be in the running set"
        );

        drop(shared);
        let event = q.rx.try_recv().expect("event should have been sent");

        assert_eq!(event.test_execution.uuid(), ex_uuid);
        assert!(matches!(event.data, EventData::CreateEnvConfigMap(_, _)));
    }

    #[tokio::test]
    #[should_panic(expected = "request_provisioning called with n_queued == 0")]
    async fn request_provisioning_panics_when_n_queued_is_zero() {
        let (_, h, _, _) = EventQueue::new(1, 5);
        let ex = TestExecution::create_stub(1, 1, "test");

        h.request_provisioning(ex, stub_test_plan()).await;
    }

    #[tokio::test]
    async fn request_provisioning_returns_false_when_channel_is_closed() {
        let (q, h, _, _) = EventQueue::new(1, 5);
        q.shared.lock().await.n_queued = 1;
        drop(q);

        let successful = h
            .request_provisioning(TestExecution::create_stub(1, 1, "test"), stub_test_plan())
            .await;

        assert!(!successful, "should have failed to send event");
    }

    #[tokio::test]
    async fn request_provisioning_waits_for_capacity() {
        let (q, h, _, _) = EventQueue::new(1, 5); // max concurrent of 1
        let blocking_id = Uuid::new_v4();

        {
            let mut shared = q.shared.lock().await;
            shared.running_executions.insert(blocking_id);
            shared.n_queued = 1;
        }

        let ex = TestExecution::create_stub(1, 1, "test");
        let provision_task =
            tokio::spawn(async move { h.request_provisioning(ex, stub_test_plan()).await });

        // yield to allow the provisioning task to run (if able)
        tokio::task::yield_now().await;
        assert!(
            !provision_task.is_finished(),
            "should be blocked waiting for capacity"
        );

        // Clearing the "running" execution should unblock the pending provisioning request
        q.mark_execution_complete(blocking_id).await;

        let successful = tokio::time::timeout(Duration::from_secs(1), provision_task)
            .await
            .expect("provision task should have unblocked within 1s")
            .expect("provision task should not have panicked");

        assert!(successful, "request_provisioning should have returned true");
    }

    fn provision_evt(ex_id: i32) -> Event {
        Event {
            test_execution: TestExecution::create_stub(ex_id, 1, "test"),
            data: EventData::CreateEnvConfigMap(stub_environment(), stub_scenario()),
        }
    }

    fn cleanup_evt(ex_id: i32) -> Event {
        Event {
            test_execution: TestExecution::create_stub(ex_id, 1, "test"),
            data: EventData::CleanupNamespace,
        }
    }

    #[test_case(vec![provision_evt(1), provision_evt(2)], 1; "only provision")]
    #[test_case(vec![cleanup_evt(1), cleanup_evt(2)], 1; "only non-provision")]
    #[test_case(vec![provision_evt(1), cleanup_evt(2)], 2; "both")]
    #[tokio::test]
    async fn next_event_returns_expected_event_from_pending(events: Vec<Event>, expected_id: i32) {
        // Handle needs to stay alive: dropping it reduces sender_strong_count to 1, causing the
        // drain loop to return None before reaching the priority selection logic.
        let (mut q, _h, _, _) = EventQueue::new(1, 5);

        for evt in events.into_iter() {
            if matches!(evt.data, EventData::CreateEnvConfigMap(_, _)) {
                q.pending_provisions.push_back(evt);
            } else {
                q.pending_non_provisions.push_back(evt);
            }
        }

        let evt = q.next_event().await.expect("should have returned an event");

        assert_eq!(
            evt.test_execution.id(),
            expected_id,
            "wrong event returned: {evt:?}"
        );
    }

    #[tokio::test]
    async fn next_event_drains_channel_before_selecting() {
        // Handle needs to stay alive: dropping it reduces sender_strong_count to 1, causing the
        // drain loop to return None before reaching the priority selection logic.
        let (mut q, _h, _, _) = EventQueue::new(1, 5);

        // Send the provision event first so we know that we're not just relying on ordering
        q.tx.send(provision_evt(1)).unwrap();
        q.tx.send(cleanup_evt(2)).unwrap();

        let evt = q.next_event().await.expect("should have returned an event");

        assert_eq!(evt.test_execution.id(), 2, "wrong event returned: {evt:?}");
    }

    #[tokio::test]
    async fn next_event_blocks_when_no_events_are_available() {
        // Handle needs to stay alive: dropping it reduces sender_strong_count to 1, causing the
        // drain loop to return None before reaching the priority selection logic.
        let (mut q, _h, _, _) = EventQueue::new(1, 5);
        let tx = q.tx.clone();

        let next_event_task = tokio::spawn(async move { q.next_event().await });

        // yield to allow the next_event task to run (if able)
        tokio::task::yield_now().await;
        assert!(
            !next_event_task.is_finished(),
            "should be blocked waiting for an event"
        );

        tx.send(cleanup_evt(1)).unwrap();

        let evt = tokio::time::timeout(Duration::from_secs(1), next_event_task)
            .await
            .expect("should have unblocked within 1s")
            .expect("task should not have panicked")
            .expect("should have returned Some(event)");

        assert_eq!(evt.test_execution.id(), 1, "wrong event returned: {evt:?}");
    }

    #[tokio::test]
    async fn next_event_returns_none_when_no_external_senders_remain() {
        let (mut q, h, _, _) = EventQueue::new(1, 5);

        // Start with an event in the channel so we skip the blocking recv call and drop into the
        // drain loop.
        q.tx.send(cleanup_evt(1)).unwrap();
        drop(h);

        assert_eq!(q.next_event().await, None, "should have returned None");
        assert_eq!(q.pending_non_provisions.len(), 0, "unexpected recv");
        assert!(!q.rx.is_empty(), "event should still be in the channel");
    }
}
