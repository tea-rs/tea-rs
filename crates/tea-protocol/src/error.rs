use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::envelope::deserialize_unique_value;
use thiserror::Error;

use crate::{
    CURRENT_PROTOCOL_VERSION, CorrelationId, ProtocolMetadata, ProtocolMetadataError,
    ProtocolVersion,
};

/// Maximum UTF-8 bytes in an English technical error message.
pub const MAX_ERROR_MESSAGE_BYTES: usize = 4096;

/// Stable machine-readable protocol error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorCode {
    /// The command discriminator is not supported by this host.
    UnsupportedCommand,
    /// A required durable record discriminator is not supported.
    UnsupportedRecord,
    /// The protocol version is outside the supported range.
    UnsupportedProtocolVersion,
    /// A known command has invalid structure or state.
    InvalidCommand,
    /// Input failed canonical validation.
    InvalidInput,
    /// An append or transition used a stale expected sequence.
    SequenceConflict,
    /// An upstream or host rate limit rejected the request.
    RateLimited,
    /// A provider is temporarily unavailable.
    ProviderUnavailable,
    /// The operation was cancelled.
    Cancelled,
    /// An unexpected internal failure occurred.
    Internal,
}

/// Whether and how a caller may retry an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryClass {
    /// Retrying the same operation is not safe or useful.
    Never,
    /// The same operation may be retried immediately.
    Immediate,
    /// The operation may be retried after bounded backoff.
    AfterBackoff,
}

/// A safe, localizable protocol error payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolError {
    code: AgentErrorCode,
    message: String,
    retry: RetryClass,
    correlation_id: Option<CorrelationId>,
    details: ProtocolMetadata,
}

impl ProtocolError {
    /// Creates a validated protocol error without serializing an internal source chain.
    ///
    /// # Errors
    ///
    /// Returns an error when the technical message is empty, contains a null
    /// character, or exceeds [`MAX_ERROR_MESSAGE_BYTES`].
    pub fn new(
        code: AgentErrorCode,
        message: impl Into<String>,
        retry: RetryClass,
    ) -> Result<Self, ProtocolErrorValidationError> {
        let message = message.into();
        validate_message(&message)?;
        Ok(Self {
            code,
            message,
            retry,
            correlation_id: None,
            details: ProtocolMetadata::default(),
        })
    }

    pub(crate) fn invalid_command(correlation_id: CorrelationId) -> Self {
        Self {
            code: AgentErrorCode::InvalidCommand,
            message: "command envelope is invalid".to_owned(),
            retry: RetryClass::Never,
            correlation_id: Some(correlation_id),
            details: ProtocolMetadata::default(),
        }
    }

    pub(crate) fn invalid_record(correlation_id: CorrelationId) -> Self {
        Self {
            code: AgentErrorCode::InvalidInput,
            message: "durable session record is invalid".to_owned(),
            retry: RetryClass::Never,
            correlation_id: Some(correlation_id),
            details: ProtocolMetadata::default(),
        }
    }

    pub(crate) fn unsupported_protocol_version(
        correlation_id: CorrelationId,
        received_version: ProtocolVersion,
    ) -> Self {
        Self {
            code: AgentErrorCode::UnsupportedProtocolVersion,
            message: "protocol version is not supported".to_owned(),
            retry: RetryClass::Never,
            correlation_id: Some(correlation_id),
            details: ProtocolMetadata::protocol_version_details(&received_version.to_string()),
        }
    }

    pub(crate) fn unsupported_record(correlation_id: CorrelationId, record_type: &str) -> Self {
        Self {
            code: AgentErrorCode::UnsupportedRecord,
            message: "durable record type is not supported".to_owned(),
            retry: RetryClass::Never,
            correlation_id: Some(correlation_id),
            details: ProtocolMetadata::protocol_compatibility_details(Some(record_type)),
        }
    }

    /// Creates the canonical unsupported-command error for protocol 1.x.
    #[must_use]
    pub fn unsupported_command(correlation_id: CorrelationId) -> Self {
        let details = ProtocolMetadata::protocol_compatibility_details(None);
        Self {
            code: AgentErrorCode::UnsupportedCommand,
            message: "command type is not supported".to_owned(),
            retry: RetryClass::Never,
            correlation_id: Some(correlation_id),
            details,
        }
    }

    /// Adds a diagnostic correlation identifier.
    #[must_use]
    pub fn with_correlation_id(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Adds validated, bounded safe details.
    #[must_use]
    pub fn with_details(mut self, details: ProtocolMetadata) -> Self {
        self.details = details;
        self
    }

    /// Returns the stable error code.
    #[must_use]
    pub const fn code(&self) -> AgentErrorCode {
        self.code
    }

    /// Returns the English technical diagnostic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the retry classification.
    #[must_use]
    pub const fn retry(&self) -> RetryClass {
        self.retry
    }

    /// Returns the optional diagnostic correlation identifier.
    #[must_use]
    pub const fn correlation_id(&self) -> Option<&CorrelationId> {
        self.correlation_id.as_ref()
    }

    /// Returns bounded safe structured details.
    #[must_use]
    pub const fn details(&self) -> &ProtocolMetadata {
        &self.details
    }

    fn validate(&self) -> Result<(), ProtocolErrorValidationError> {
        validate_message(&self.message)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SerializableProtocolError<'a> {
    code: AgentErrorCode,
    message: &'a str,
    retry: RetryClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    correlation_id: &'a Option<CorrelationId>,
    #[serde(skip_serializing_if = "ProtocolMetadata::is_empty")]
    details: &'a ProtocolMetadata,
}

impl Serialize for ProtocolError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        SerializableProtocolError {
            code: self.code,
            message: &self.message,
            retry: self.retry,
            correlation_id: &self.correlation_id,
            details: &self.details,
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProtocolError {
    code: AgentErrorCode,
    message: String,
    retry: RetryClass,
    #[serde(default)]
    correlation_id: Option<CorrelationId>,
    #[serde(default)]
    details: ProtocolMetadata,
}

impl<'de> Deserialize<'de> for ProtocolError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawProtocolError::deserialize(deserializer)?;
        let mut error = Self::new(raw.code, raw.message, raw.retry)
            .map_err(serde::de::Error::custom)?
            .with_details(raw.details);
        error.correlation_id = raw.correlation_id;
        Ok(error)
    }
}

/// Versioned protocol-error transport envelope.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolErrorEnvelope {
    protocol_version: ProtocolVersion,
    #[serde(rename = "type")]
    kind: ProtocolErrorEnvelopeType,
    error: ProtocolError,
}

impl ProtocolErrorEnvelope {
    /// Wraps an error in the current protocol envelope.
    #[must_use]
    pub const fn new(error: ProtocolError) -> Self {
        Self {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            kind: ProtocolErrorEnvelopeType::ProtocolError,
            error,
        }
    }

    /// Returns the envelope protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Returns the safe error payload.
    #[must_use]
    pub const fn error(&self) -> &ProtocolError {
        &self.error
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProtocolErrorEnvelopeType {
    ProtocolError,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProtocolErrorEnvelope {
    protocol_version: ProtocolVersion,
    #[serde(rename = "type")]
    kind: ProtocolErrorEnvelopeType,
    error: ProtocolError,
}

impl<'de> Deserialize<'de> for ProtocolErrorEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_unique_value(deserializer)?;
        let raw = RawProtocolErrorEnvelope::deserialize(value).map_err(serde::de::Error::custom)?;
        if raw.protocol_version.major() != CURRENT_PROTOCOL_VERSION.major() {
            return Err(serde::de::Error::custom(
                "unsupported protocol major version",
            ));
        }
        Ok(Self {
            protocol_version: raw.protocol_version,
            kind: raw.kind,
            error: raw.error,
        })
    }
}

/// Error returned when validating a safe protocol error.
#[derive(Debug, Error)]
pub enum ProtocolErrorValidationError {
    /// The technical message is empty, oversized, or contains a null character.
    #[error("technical error message is invalid")]
    InvalidMessage,
    /// Safe details failed metadata validation.
    #[error("safe error details are invalid: {0}")]
    InvalidDetails(#[from] ProtocolMetadataError),
}

fn validate_message(message: &str) -> Result<(), ProtocolErrorValidationError> {
    if message.is_empty()
        || message.len() > MAX_ERROR_MESSAGE_BYTES
        || message.contains('\0')
        || message.chars().any(|character| character == '\r')
    {
        Err(ProtocolErrorValidationError::InvalidMessage)
    } else {
        Ok(())
    }
}
