use serde_json::Value;
use tea_protocol::{ContentBlock, ProtocolMetadata, ToolPresentation, Usage};
use thiserror::Error;

/// Stable tool execution failure code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionFailureCode {
    /// Executor reported an expected operation failure.
    ExecutionFailed,
    /// Invocation was cooperatively cancelled.
    Cancelled,
    /// Executor violated its output contract.
    InvalidOutput,
    /// Executor failed internally.
    Internal,
}

/// Bounded machine-readable tool execution failure.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecutionFailure {
    code: ToolExecutionFailureCode,
    message: String,
    details: ProtocolMetadata,
}

impl ToolExecutionFailure {
    /// Creates an execution failure.
    ///
    /// # Errors
    ///
    /// Returns an error when the technical message is invalid.
    pub fn execution(message: impl Into<String>) -> Result<Self, ToolResultError> {
        Self::new(ToolExecutionFailureCode::ExecutionFailed, message)
    }
    /// Creates a cancellation failure.
    #[must_use]
    pub fn cancelled() -> Self {
        Self {
            code: ToolExecutionFailureCode::Cancelled,
            message: "tool execution was cancelled".to_owned(),
            details: ProtocolMetadata::default(),
        }
    }
    /// Creates an invalid-output contract failure.
    #[must_use]
    pub fn invalid_output() -> Self {
        Self {
            code: ToolExecutionFailureCode::InvalidOutput,
            message: "tool executor returned invalid output".to_owned(),
            details: ProtocolMetadata::default(),
        }
    }
    /// Creates a fixed internal executor-contract failure.
    #[must_use]
    pub fn internal_contract() -> Self {
        Self {
            code: ToolExecutionFailureCode::Internal,
            message: "tool executor stream ended without a terminal result".to_owned(),
            details: ProtocolMetadata::default(),
        }
    }
    fn new(
        code: ToolExecutionFailureCode,
        message: impl Into<String>,
    ) -> Result<Self, ToolResultError> {
        let message = message.into();
        if message.is_empty() || message.len() > 4096 || message.contains('\0') {
            return Err(ToolResultError::InvalidFailureMessage);
        }
        Ok(Self {
            code,
            message,
            details: ProtocolMetadata::default(),
        })
    }
    /// Adds bounded namespaced safe details.
    #[must_use]
    pub fn with_details(mut self, details: ProtocolMetadata) -> Self {
        self.details = details;
        self
    }
    /// Returns failure code.
    #[must_use]
    pub const fn code(&self) -> ToolExecutionFailureCode {
        self.code
    }
    /// Returns English technical message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    /// Returns safe details.
    #[must_use]
    pub const fn details(&self) -> &ProtocolMetadata {
        &self.details
    }
}

/// Successful terminal tool result.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    content: Vec<ContentBlock>,
    output: Value,
    details: ProtocolMetadata,
    presentation: Option<ToolPresentation>,
    usage: Option<Usage>,
}

impl ToolResult {
    /// Creates a result with model-visible content and machine output.
    ///
    /// # Errors
    ///
    /// Returns an error for empty/invalid content or non-object output.
    pub fn new(content: Vec<ContentBlock>, output: Value) -> Result<Self, ToolResultError> {
        if content.is_empty()
            || content.len() > 256
            || !output.is_object()
            || serde_json::to_vec(&output)
                .map_err(|_| ToolResultError::InvalidResult)?
                .len()
                > 256 * 1024
            || json_depth(&output) > 32
        {
            return Err(ToolResultError::InvalidResult);
        }
        if content.iter().any(|block| {
            !matches!(
                block,
                ContentBlock::Text { .. } | ContentBlock::Image { .. }
            )
        }) || content
            .iter()
            .any(|block| serde_json::to_value(block).is_err())
        {
            return Err(ToolResultError::InvalidResult);
        }
        Ok(Self {
            content,
            output,
            details: ProtocolMetadata::default(),
            presentation: None,
            usage: None,
        })
    }
    /// Adds safe details.
    #[must_use]
    pub fn with_details(mut self, details: ProtocolMetadata) -> Self {
        self.details = details;
        self
    }
    /// Adds a bounded durable UI presentation kept out of model-visible content.
    #[must_use]
    pub fn with_presentation(mut self, presentation: ToolPresentation) -> Self {
        self.presentation = Some(presentation);
        self
    }
    /// Adds tool-specific usage.
    #[must_use]
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }
    /// Returns model-visible content.
    #[must_use]
    pub fn content(&self) -> &[ContentBlock] {
        &self.content
    }
    /// Returns machine output.
    #[must_use]
    pub const fn output(&self) -> &Value {
        &self.output
    }
    /// Returns safe details.
    #[must_use]
    pub const fn details(&self) -> &ProtocolMetadata {
        &self.details
    }
    /// Returns the optional durable UI presentation.
    #[must_use]
    pub const fn presentation(&self) -> Option<&ToolPresentation> {
        self.presentation.as_ref()
    }
    /// Returns tool-specific usage.
    #[must_use]
    pub const fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }
}

/// Error constructing tool results/failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolResultError {
    /// Result content or output is invalid.
    #[error("tool result is invalid")]
    InvalidResult,
    /// Failure message is invalid.
    #[error("tool failure message is invalid")]
    InvalidFailureMessage,
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}
