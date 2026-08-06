use tea_protocol::{
    AgentEvent, EventEnvelope, ProtocolMetadata, RunId, SessionId, SessionSequence, TurnId,
};

use crate::{
    KernelClock, KernelError, KernelErrorCode, KernelEventSink, KernelIdSource, RunLimits,
};

pub(crate) struct EventEmitter<'a> {
    ids: &'a dyn KernelIdSource,
    clock: &'a dyn KernelClock,
    sink: &'a dyn KernelEventSink,
    session_id: SessionId,
    sequence: SessionSequence,
    emitted: u64,
    limits: RunLimits,
}

impl<'a> EventEmitter<'a> {
    pub(crate) fn new(
        ids: &'a dyn KernelIdSource,
        clock: &'a dyn KernelClock,
        sink: &'a dyn KernelEventSink,
        session_id: SessionId,
        durable_tail: SessionSequence,
        limits: RunLimits,
    ) -> Self {
        Self {
            ids,
            clock,
            sink,
            session_id,
            sequence: match sink.last_sequence(session_id) {
                Some(cursor) if cursor > durable_tail => cursor,
                Some(_) | None => durable_tail,
            },
            emitted: 0,
            limits,
        }
    }

    pub(crate) async fn emit(
        &mut self,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        event: AgentEvent,
    ) -> Result<(), KernelError> {
        if self.emitted >= self.limits.max_events() {
            return Err(KernelError::new(
                KernelErrorCode::LimitExceeded,
                "run event limit was reached",
            ));
        }
        self.sequence = self.sequence.checked_next().ok_or_else(|| {
            KernelError::new(
                KernelErrorCode::LimitExceeded,
                "event sequence cannot advance",
            )
        })?;
        let envelope = EventEnvelope::new(
            self.ids.next_event_id()?,
            self.session_id,
            run_id,
            turn_id,
            self.sequence,
            self.clock.now()?,
            ProtocolMetadata::default(),
            event,
        )
        .map_err(|error| KernelError::new(KernelErrorCode::InvalidState, error.to_string()))?;
        self.sink.emit(envelope).await.map_err(|error| {
            KernelError::new(KernelErrorCode::EventSinkFailure, error.to_string())
        })?;
        self.emitted += 1;
        Ok(())
    }
}
