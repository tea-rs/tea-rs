use serde::{Deserialize, Serialize};
use tea_protocol::{RecordId, SessionId, SessionSequence};
use thiserror::Error;

/// Stable storage failure classification shared by session-store adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStoreErrorCode {
    /// The requested session does not exist.
    SessionNotFound,
    /// Creation was requested for an existing session.
    SessionAlreadyExists,
    /// A writer supplied a stale or otherwise incorrect expected sequence.
    SequenceConflict,
    /// A durable record is malformed for the session state machine.
    InvalidRecord,
    /// A durable record references an unknown or incompatible entity.
    InvalidReference,
    /// The record or archive schema version is unsupported.
    UnsupportedSchemaVersion,
    /// Stored source facts violate append-only log invariants.
    CorruptionDetected,
    /// An atomic transaction could not be committed.
    TransactionFailed,
    /// The storage adapter is unavailable.
    StorageUnavailable,
}

/// Deterministic failure while reducing durable records.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionReplayError {
    /// Replay requires at least one creation record.
    #[error("session log is empty")]
    EmptyLog,
    /// The first record is not the sole session creation record.
    #[error("session creation record is missing or duplicated")]
    InvalidCreation,
    /// One envelope belongs to another session.
    #[error("record belongs to session {actual}, expected {expected}")]
    SessionMismatch {
        /// Expected session identity.
        expected: SessionId,
        /// Actual envelope session identity.
        actual: SessionId,
    },
    /// Authoritative sequence is not contiguous.
    #[error("record sequence is {actual}, expected {expected}")]
    SequenceMismatch {
        /// Required next sequence.
        expected: SessionSequence,
        /// Actual sequence.
        actual: SessionSequence,
    },
    /// Sequence cannot advance beyond its integer representation.
    #[error("session sequence overflow")]
    SequenceOverflow,
    /// Record identity was already present in the log.
    #[error("duplicate record ID: {record_id}")]
    DuplicateRecord {
        /// Reused record identity.
        record_id: RecordId,
    },
    /// A globally stable entity identity was reused.
    #[error("duplicate session entity: {entity}")]
    DuplicateEntity {
        /// Bounded technical entity category.
        entity: &'static str,
    },
    /// A record references an entity that is absent or in the wrong state.
    #[error("invalid session reference: {reference}")]
    InvalidReference {
        /// Bounded technical reference category.
        reference: &'static str,
    },
    /// A known transition is invalid for the current state.
    #[error("invalid session transition: {transition}")]
    InvalidTransition {
        /// Bounded technical transition category.
        transition: &'static str,
    },
}

impl SessionReplayError {
    /// Maps replay failures to a stable storage-facing classification.
    #[must_use]
    pub const fn store_code(&self) -> SessionStoreErrorCode {
        match self {
            Self::InvalidReference { .. } => SessionStoreErrorCode::InvalidReference,
            Self::SequenceMismatch { .. }
            | Self::SequenceOverflow
            | Self::DuplicateRecord { .. }
            | Self::DuplicateEntity { .. }
            | Self::SessionMismatch { .. }
            | Self::EmptyLog
            | Self::InvalidCreation => SessionStoreErrorCode::CorruptionDetected,
            Self::InvalidTransition { .. } => SessionStoreErrorCode::InvalidRecord,
        }
    }
}

/// Stable session repository failure with an English technical diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct SessionStoreError {
    code: SessionStoreErrorCode,
    message: String,
}

impl SessionStoreError {
    /// Creates a bounded storage error.
    #[must_use]
    pub fn new(code: SessionStoreErrorCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
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

    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(&self) -> SessionStoreErrorCode {
        self.code
    }

    /// Returns the bounded English technical diagnostic.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<SessionReplayError> for SessionStoreError {
    fn from(error: SessionReplayError) -> Self {
        Self::new(error.store_code(), error.to_string())
    }
}
