use std::fmt::Debug;
use std::pin::Pin;

use futures_core::Stream;
use tea_control::CancellationScope;
use tea_protocol::ProtocolMetadata;
use thiserror::Error;

use crate::{ToolExecutionFailure, ToolResult, ValidatedToolInvocation};
use tea_protocol::ToolPresentation;

/// Non-durable tool execution progress.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolProgress {
    message: String,
    completed_units: u64,
    total_units: Option<u64>,
    details: ProtocolMetadata,
}

impl ToolProgress {
    /// Creates bounded monotonic progress metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid messages or completed units above total.
    pub fn new(
        message: impl Into<String>,
        completed_units: u64,
        total_units: Option<u64>,
    ) -> Result<Self, ToolStreamViolation> {
        let message = message.into();
        if message.is_empty()
            || message.len() > 4096
            || message.contains('\0')
            || total_units.is_some_and(|total| completed_units > total)
        {
            return Err(ToolStreamViolation::InvalidProgress);
        }
        Ok(Self {
            message,
            completed_units,
            total_units,
            details: ProtocolMetadata::default(),
        })
    }
    /// Returns technical progress message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    /// Adds bounded safe progress details.
    #[must_use]
    pub fn with_details(mut self, details: ProtocolMetadata) -> Self {
        self.details = details;
        self
    }
    /// Returns completed units.
    #[must_use]
    pub const fn completed_units(&self) -> u64 {
        self.completed_units
    }
    /// Returns total units.
    #[must_use]
    pub const fn total_units(&self) -> Option<u64> {
        self.total_units
    }
    /// Returns bounded safe progress details.
    #[must_use]
    pub const fn details(&self) -> &ProtocolMetadata {
        &self.details
    }
}

/// One normalized tool execution stream event.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolExecutionEvent {
    /// Non-durable progress.
    Progress(ToolProgress),
    /// Successful terminal result.
    Finished(ToolResult),
    /// Failed or cancelled terminal result.
    Failed(ToolExecutionFailure),
}

/// Provider-neutral asynchronous tool event stream.
pub trait ToolExecutionStream: Stream<Item = ToolExecutionEvent> + Send {}
impl<T> ToolExecutionStream for T where T: Stream<Item = ToolExecutionEvent> + Send {}
/// Object-safe boxed tool execution stream.
pub type BoxToolExecutionStream = Pin<Box<dyn ToolExecutionStream + 'static>>;

/// Object-safe executor receiving only registry-validated invocations.
pub trait ToolExecutor: Debug + Send + Sync {
    /// Produces an optional, non-durable preview for one validated invocation.
    ///
    /// Implementations must not mutate external state. A missing preview is
    /// intentionally indistinguishable from an unavailable preview so callers
    /// can preserve the normal approval and execution flow.
    fn preview(&self, _invocation: &ValidatedToolInvocation) -> Option<ToolPresentation> {
        None
    }

    /// Creates a lazy execution stream owned by the caller.
    fn execute(
        &self,
        invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream;
}

/// Deterministic tool stream grammar validator.
#[derive(Debug, Default)]
pub struct ToolStreamValidator {
    terminal: bool,
    events: usize,
}
impl ToolStreamValidator {
    /// Creates an empty validator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Observes one event.
    ///
    /// # Errors
    ///
    /// Returns an error when an event follows a terminal event.
    pub fn observe(&mut self, event: &ToolExecutionEvent) -> Result<(), ToolStreamViolation> {
        if self.terminal {
            return Err(ToolStreamViolation::EventAfterTerminal);
        }
        if matches!(
            event,
            ToolExecutionEvent::Finished(_) | ToolExecutionEvent::Failed(_)
        ) {
            self.terminal = true;
        }
        self.events += 1;
        Ok(())
    }
    /// Finishes after stream end.
    ///
    /// # Errors
    ///
    /// Returns an error when no terminal event was observed.
    pub fn finish(self) -> Result<usize, ToolStreamViolation> {
        if self.terminal {
            Ok(self.events)
        } else {
            Err(ToolStreamViolation::MissingTerminal)
        }
    }
}

/// Tool execution stream grammar violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolStreamViolation {
    /// Progress is invalid.
    #[error("tool progress is invalid")]
    InvalidProgress,
    /// Event followed terminal result.
    #[error("tool event appeared after terminal")]
    EventAfterTerminal,
    /// Stream ended without terminal result.
    #[error("tool stream ended without terminal result")]
    MissingTerminal,
}
