use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable machine-readable context failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextErrorCode {
    /// An identifier, provenance value, or enum relationship is invalid.
    InvalidValue,
    /// A module or segment exceeds deterministic bounds.
    BoundsExceeded,
    /// The same canonical identity was defined inconsistently.
    DuplicateIdentity,
    /// Equal-precedence conflict claims are ambiguous.
    AmbiguousConflict,
    /// Required prompt content does not fit the configured budget.
    BudgetExceeded,
    /// A context provider failed without producing usable modules.
    ProviderFailure,
}

/// Bounded safe context/compiler failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct ContextError {
    code: ContextErrorCode,
    message: String,
}

impl ContextError {
    /// Creates a bounded English technical context error.
    #[must_use]
    pub fn new(code: ContextErrorCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.is_empty() {
            "context operation failed".clone_into(&mut message);
        }
        message.retain(|character| character != '\0');
        if message.len() > 4096 {
            let boundary = message
                .char_indices()
                .map(|(index, _)| index)
                .take_while(|index| *index <= 4096)
                .last()
                .unwrap_or(0);
            message.truncate(boundary);
        }
        Self { code, message }
    }

    /// Returns the stable classification.
    #[must_use]
    pub const fn code(&self) -> ContextErrorCode {
        self.code
    }

    /// Returns the bounded safe diagnostic.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}
