use std::time::Duration;

use tea_protocol::{ProtocolMetadata, RetryClass};

use crate::ModelStreamValueError;

/// Stable provider-neutral model failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFailureCode {
    /// Request failed provider-neutral validation.
    InvalidRequest,
    /// Prompt and requested output exceed model context.
    ContextOverflow,
    /// Provider credentials are missing or invalid.
    Authentication,
    /// Credentials are valid but operation is not permitted.
    PermissionDenied,
    /// Provider rate limit rejected the operation.
    RateLimited,
    /// Provider or selected model is temporarily unavailable.
    Unavailable,
    /// Network or transport failed.
    Transport,
    /// Provider response could not be normalized safely.
    MalformedResponse,
    /// Operation was cooperatively cancelled.
    Cancelled,
    /// Unexpected adapter/runtime failure.
    Internal,
}

impl ModelFailureCode {
    /// All stable failure codes.
    pub const ALL: [Self; 10] = [
        Self::InvalidRequest,
        Self::ContextOverflow,
        Self::Authentication,
        Self::PermissionDenied,
        Self::RateLimited,
        Self::Unavailable,
        Self::Transport,
        Self::MalformedResponse,
        Self::Cancelled,
        Self::Internal,
    ];
}

/// Provider-neutral terminal model failure.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelFailure {
    code: ModelFailureCode,
    message: String,
    retry: RetryClass,
    retry_after: Option<Duration>,
    metadata: ProtocolMetadata,
    safe_diagnostic: bool,
}

impl ModelFailure {
    /// Creates a fixed internal adapter failure.
    #[must_use]
    pub fn internal_adapter_failure() -> Self {
        Self {
            code: ModelFailureCode::Internal,
            message: "model adapter failed internally".to_owned(),
            retry: RetryClass::Never,
            retry_after: None,
            metadata: ProtocolMetadata::default(),
            safe_diagnostic: false,
        }
    }

    /// Creates a bounded technical failure without an internal source chain.
    ///
    /// # Errors
    ///
    /// Returns an error when `message` is empty, exceeds 4 KiB, or contains a
    /// null character.
    pub fn new(
        code: ModelFailureCode,
        message: impl Into<String>,
        retry: RetryClass,
    ) -> Result<Self, ModelStreamValueError> {
        let message = message.into();
        if message.is_empty() || message.len() > 4096 || message.contains('\0') {
            return Err(ModelStreamValueError::InvalidFailureMessage);
        }
        Ok(Self {
            code,
            message,
            retry,
            retry_after: None,
            metadata: ProtocolMetadata::default(),
            safe_diagnostic: false,
        })
    }

    /// Creates a provider failure whose message was normalized for display.
    ///
    /// The caller must have removed provider payload fields, bounded the text,
    /// and stripped terminal control characters before using this constructor.
    ///
    /// # Errors
    ///
    /// Returns an error when the normalized message violates the model failure
    /// bounds.
    pub fn safe(
        code: ModelFailureCode,
        message: impl Into<String>,
        retry: RetryClass,
    ) -> Result<Self, ModelStreamValueError> {
        let mut failure = Self::new(code, message, retry)?;
        failure.safe_diagnostic = true;
        Ok(failure)
    }

    /// Adds bounded namespaced safe metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: ProtocolMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Adds a provider-requested delay before this failure is retried.
    #[must_use]
    pub fn with_retry_after(mut self, retry_after: Duration) -> Self {
        self.retry_after = Some(retry_after);
        self
    }

    /// Returns the stable failure code.
    #[must_use]
    pub const fn code(&self) -> ModelFailureCode {
        self.code
    }

    /// Returns the English technical message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the retry classification.
    #[must_use]
    pub const fn retry(&self) -> RetryClass {
        self.retry
    }

    /// Returns the provider-requested retry delay when one was supplied.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// Returns safe namespaced metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ProtocolMetadata {
        &self.metadata
    }

    /// Returns whether the message is safe to expose as provider diagnostics.
    #[must_use]
    pub const fn is_safe_diagnostic(&self) -> bool {
        self.safe_diagnostic
    }
}
