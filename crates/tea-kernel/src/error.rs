use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable machine-readable kernel failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelErrorCode {
    /// The requested model is absent or incompatible with the run snapshot.
    InvalidModel,
    /// Run configuration or immutable request construction failed.
    InvalidRequest,
    /// A runtime state transition is not legal.
    InvalidState,
    /// A model adapter stream violated its normalized contract.
    ModelFailure,
    /// Tool lookup, validation, or execution failed.
    ToolFailure,
    /// Policy or approval context could not be constructed safely.
    PolicyFailure,
    /// Durable session state could not be loaded or appended.
    SessionFailure,
    /// The awaited observation sink rejected an event.
    EventSinkFailure,
    /// The run was cooperatively cancelled.
    Cancelled,
    /// A deterministic run limit was reached.
    LimitExceeded,
    /// A required deterministic ID could not be produced.
    IdExhausted,
    /// The configured clock could not provide a canonical timestamp.
    ClockFailure,
    /// The compiled prompt, tools, and messages exceed the model context window.
    ContextOverflow,
    /// A retryable model request exhausted the configured retry policy.
    RetryExhausted,
    /// The tool scheduler could not place an invocation safely.
    SchedulerConflict,
}

/// Bounded safe failure returned by the agent kernel.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct KernelError {
    code: KernelErrorCode,
    message: String,
    safe_diagnostic: bool,
}

impl KernelError {
    /// Creates a bounded English technical failure.
    #[must_use]
    pub fn new(code: KernelErrorCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.is_empty() {
            "kernel operation failed".clone_into(&mut message);
        }
        if message.len() > 4096 {
            let boundary = message
                .char_indices()
                .map(|(index, _)| index)
                .take_while(|index| *index <= 4096)
                .last()
                .unwrap_or(0);
            message.truncate(boundary);
        }
        message.retain(|character| character != '\0');
        Self {
            code,
            message,
            safe_diagnostic: false,
        }
    }

    /// Creates a provider failure whose message was normalized by an adapter.
    #[must_use]
    pub fn provider_failure(code: KernelErrorCode, message: impl Into<String>) -> Self {
        let mut error = Self::new(code, message);
        error.safe_diagnostic = true;
        error
    }

    /// Returns the stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> KernelErrorCode {
        self.code
    }

    /// Returns the bounded safe diagnostic.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether the message is safe to expose as provider diagnostics.
    #[must_use]
    pub const fn is_safe_diagnostic(&self) -> bool {
        self.safe_diagnostic
    }
}
