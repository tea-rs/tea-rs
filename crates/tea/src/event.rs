use std::collections::HashMap;
use std::sync::Mutex;

use tea_kernel::{KernelEventFuture, KernelEventSink};
use tea_protocol::{EventEnvelope, SessionId, SessionSequence};
use tokio::sync::mpsc;

/// Bound applied to each per-session subscription channel.
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 256;
/// Maximum subscribers permitted for one session.
pub const MAX_EVENT_SUBSCRIBERS: usize = 64;

/// Awaited fan-out event sink backing the runtime subscription API.
///
/// Emits to every subscriber for the event's session. A full channel applies
/// backpressure (the awaited `emit` blocks the run); a closed channel is
/// dropped from the subscriber list. A session with no subscribers accepts an
/// event without backpressure.
#[derive(Debug)]
pub struct RuntimeEventSink {
    capacity: usize,
    state: Mutex<SinkState>,
}

#[derive(Debug, Default)]
struct SinkState {
    subscribers: HashMap<SessionId, Vec<mpsc::Sender<EventEnvelope>>>,
    last_sequence: HashMap<SessionId, SessionSequence>,
}

impl RuntimeEventSink {
    /// Creates a sink with a bounded per-subscriber channel capacity.
    ///
    /// # Panics
    ///
    /// Panics only if constructed through internal helpers with a zero capacity.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_EVENT_CHANNEL_CAPACITY)
    }

    /// Creates a sink with a custom bounded channel capacity.
    ///
    /// # Panics
    ///
    /// Panics for a zero capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0, "event channel capacity must be non-zero");
        Self {
            capacity,
            state: Mutex::new(SinkState::default()),
        }
    }

    /// Subscribes to events for one session, returning a bounded receiver.
    ///
    /// # Errors
    ///
    /// Returns an error when too many subscribers are already registered.
    pub fn subscribe(
        &self,
        session_id: SessionId,
    ) -> Result<mpsc::Receiver<EventEnvelope>, tea_kernel::KernelError> {
        let (sender, receiver) = mpsc::channel(self.capacity);
        let mut state = self.lock()?;
        let subscribers = state.subscribers.entry(session_id).or_default();
        if subscribers.len() >= MAX_EVENT_SUBSCRIBERS {
            return Err(tea_kernel::KernelError::new(
                tea_kernel::KernelErrorCode::InvalidRequest,
                "too many event subscribers for one session",
            ));
        }
        subscribers.push(sender);
        Ok(receiver)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SinkState>, tea_kernel::KernelError> {
        self.state.lock().map_err(|_| {
            tea_kernel::KernelError::new(
                tea_kernel::KernelErrorCode::InvalidState,
                "event sink state is poisoned",
            )
        })
    }
}

impl Default for RuntimeEventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelEventSink for RuntimeEventSink {
    fn last_sequence(&self, session_id: SessionId) -> Option<SessionSequence> {
        self.state
            .lock()
            .ok()?
            .last_sequence
            .get(&session_id)
            .copied()
    }

    fn emit(&self, event: EventEnvelope) -> KernelEventFuture<'_> {
        Box::pin(async move {
            let session_id = event.session_id();
            let sequence = event.sequence();
            let senders = {
                let mut state = self.lock()?;
                let sequence_entry = state.last_sequence.entry(session_id).or_insert(sequence);
                if sequence > *sequence_entry {
                    *sequence_entry = sequence;
                }
                state
                    .subscribers
                    .get(&session_id)
                    .cloned()
                    .unwrap_or_default()
            };
            let mut failed = false;
            for sender in senders {
                if sender.send(event.clone()).await.is_err() {
                    failed = true;
                }
            }
            if failed {
                self.remove_closed(session_id)?;
            }
            Ok(())
        })
    }
}

impl RuntimeEventSink {
    /// Removes subscribers that have closed their receiver for one session.
    fn remove_closed(&self, session_id: SessionId) -> Result<(), tea_kernel::KernelError> {
        let mut state = self.lock()?;
        if let Some(subscribers) = state.subscribers.get_mut(&session_id) {
            subscribers.retain(|sender| !sender.is_closed());
            if subscribers.is_empty() {
                state.subscribers.remove(&session_id);
            }
        }
        Ok(())
    }
}
