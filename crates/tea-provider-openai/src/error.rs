//! Stable failure classification for the `OpenAI` adapter.

use thiserror::Error;

/// Stable machine-readable failure classification for the `OpenAI` adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiErrorCode {
    /// The request payload was invalid.
    InvalidRequest,
    /// Credentials are missing or rejected.
    Authentication,
    /// Credentials are valid but the operation is not permitted.
    PermissionDenied,
    /// The provider rate-limited the request.
    RateLimited,
    /// The provider or model is temporarily unavailable.
    Unavailable,
    /// A network or transport error occurred.
    Transport,
    /// The provider response could not be normalized safely.
    MalformedResponse,
    /// The prompt and output exceed the model context window.
    ContextOverflow,
    /// The operation was cooperatively cancelled.
    Cancelled,
    /// An unexpected adapter failure.
    Internal,
}

/// Bounded safe failure returned by the `OpenAI` adapter.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct OpenAiError {
    code: OpenAiErrorCode,
    message: String,
}

impl OpenAiError {
    /// Creates a bounded English technical failure.
    #[must_use]
    pub fn new(code: OpenAiErrorCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.is_empty() {
            "openai adapter operation failed".clone_into(&mut message);
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
    pub const fn code(&self) -> OpenAiErrorCode {
        self.code
    }

    /// Returns the bounded safe diagnostic (never the API key).
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}
