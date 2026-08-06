use std::future::Future;
use std::pin::Pin;

use tea_protocol::{EventEnvelope, SessionId, SessionSequence};

use crate::KernelError;

/// Boxed future returned by an awaited event sink.
pub type KernelEventFuture<'a> = Pin<Box<dyn Future<Output = Result<(), KernelError>> + Send + 'a>>;

/// Awaited observation destination with explicit backpressure.
pub trait KernelEventSink: std::fmt::Debug + Send + Sync {
    /// Returns the last accepted sequence for one continuing observation stream.
    ///
    /// A sink that does not preserve cursors may return `None`; a continuing
    /// host stream should retain this value across fresh kernel instances.
    fn last_sequence(&self, _session_id: SessionId) -> Option<SessionSequence> {
        None
    }

    /// Accepts one canonical event after every prior event has completed.
    fn emit(&self, event: EventEnvelope) -> KernelEventFuture<'_>;
}

/// Event sink that intentionally discards observations after awaiting the call.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardEventSink;

impl KernelEventSink for DiscardEventSink {
    fn emit(&self, _event: EventEnvelope) -> KernelEventFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}
