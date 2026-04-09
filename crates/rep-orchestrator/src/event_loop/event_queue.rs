#![expect(dead_code)]
use crate::{
    db::TestExecution,
    event_loop::{Event, EventData},
};
use rep_orchestrator_shared::test_plan::RepTestPlan;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};
use tokio::sync::{
    Mutex, Notify,
    mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel},
};
use tracing::{error, info};
use uuid::Uuid;

/// Coordinates queuing of k8s events to provide back pressure and prioritise running executions
/// over newly submitted ones.
///
/// Held by the event loop task with paired [EventQueueHandle] and [EventQueueState] structs that
/// are used elsewhere in the codebase to submit events to the queue and introspect the current
/// queue state.
#[derive(Debug)]
pub struct EventQueue {
    /// Sender for submitting events back to the queue
    pub(crate) tx: UnboundedSender<Event>,
    /// Receiver for accepting new events
    pub(crate) rx: UnboundedReceiver<Event>,
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
    ) -> (Self, ProvisioningHandle, EventQueueState) {
        let shared = Arc::new(Mutex::new(Shared {
            max_concurrent_executions,
            max_queued_executions,
            running_executions: HashSet::new(),
            n_queued: 0,
        }));
        let notify = Arc::new(Notify::new());
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
        };

        (eq, ph, eqs)
    }

    #[inline(always)]
    fn push_event(&mut self, evt: Event) {
        match &evt.data {
            EventData::ProvisionEnvironment(_, _) => self.pending_provisions.push_back(evt),
            _ => self.pending_non_provisions.push_back(evt),
        }
    }

    /// Wait for the next [Event] to be received, prioritising non-provision events over
    /// provisioning new namespaces.
    ///
    /// Returns [None] when the event channel is closed as per [UnboundedReceiver::recv].
    pub async fn next_event(&mut self) -> Option<Event> {
        // If our channel is empty and we have nothing queued we block and wait for the next event
        // to arrive.
        if self.rx.is_empty()
            && self.pending_provisions.is_empty()
            && self.pending_non_provisions.is_empty()
        {
            let evt = self.rx.recv().await?;
            self.push_event(evt);
        }

        // Drain any pending events from the channel before we determine which event to handle next
        loop {
            match self.rx.try_recv() {
                Ok(evt) => self.push_event(evt),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return None,
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
    /// namespace slot is now available via [ProvisioningHandle::wait_for_namespace_capacity].
    pub async fn mark_execution_complete(&self, execution_id: Uuid) {
        // Ensure that we release the mutex before notifying the resolver task.
        {
            let mut shared = self.shared.lock().await;
            shared.running_executions.remove(&execution_id);
        }

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
                    "currenly have {current} active namespaces but should be capped at a max of {max}"
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
        // re-aquiring the lock as we are the only task attempting to provision executions.
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
            data: EventData::ProvisionEnvironment(tp.environment.execution, tp.scenario.execution),
        }) {
            // If the channel is closed then we're shutting down so dropping the event details here
            // is intentional.
            error!(%e, "event loop channel closed");
            return false;
        }

        true
    }
}

/// Access to shared [EventQueue] state.
///
/// Used to atomically reserve space for queuing new test executions in the resolver task.
#[derive(Debug, Clone)]
pub struct EventQueueState {
    shared: Arc<Mutex<Shared>>,
}

impl EventQueueState {
    /// Attempt to reserve the requested number of executions if there is capacity.
    ///
    /// Returns `true` if the claim was successful, otherwise `false`.
    pub async fn try_reserve_pending_executions(&self, n: usize) -> bool {
        let mut shared = self.shared.lock().await;

        if shared.n_queued.saturating_add(n) <= shared.max_queued_executions {
            shared.n_queued += n;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
struct Shared {
    max_concurrent_executions: usize,
    max_queued_executions: usize,
    running_executions: HashSet<Uuid>,
    n_queued: usize,
}
