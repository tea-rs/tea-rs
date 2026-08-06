use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ProfileSchemaVersion;
use crate::identity::ProfileTextError;

/// Stable machine-readable profile failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileErrorCode {
    /// The schema version is unsupported or mismatched.
    InvalidVersion,
    /// A selector, identifier, or enum relationship is invalid.
    InvalidSelector,
    /// A field exceeds a deterministic bound.
    BoundsExceeded,
    /// The same identity was declared more than once.
    DuplicateEntry,
    /// A composition overlay conflicts with its base profile.
    CompositionConflict,
    /// A duration, count, byte, or selector value is unsupported.
    UnsupportedValue,
}

/// Bounded safe profile failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct ProfileError {
    code: ProfileErrorCode,
    message: String,
}

impl ProfileError {
    /// Creates a bounded English technical profile error.
    #[must_use]
    pub fn new(code: ProfileErrorCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.is_empty() {
            "profile operation failed".clone_into(&mut message);
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

    /// Creates an `InvalidVersion` error for an unsupported schema version.
    #[must_use]
    pub fn from_unsupported(version: ProfileSchemaVersion) -> Self {
        Self::new(
            ProfileErrorCode::InvalidVersion,
            format!("unsupported profile schema version {version}"),
        )
    }

    /// Returns the stable classification.
    #[must_use]
    pub const fn code(&self) -> ProfileErrorCode {
        self.code
    }

    /// Returns the bounded safe diagnostic.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<ProfileTextError> for ProfileError {
    fn from(_: ProfileTextError) -> Self {
        Self::new(
            ProfileErrorCode::InvalidSelector,
            "profile text must be non-empty, bounded, and null-free",
        )
    }
}
