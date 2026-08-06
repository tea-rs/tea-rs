use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A monotonically increasing, session-local record or event sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionSequence(u64);

impl SessionSequence {
    /// Creates a sequence from its integer representation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next sequence, or `None` on integer overflow.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Error returned when parsing a session sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SessionSequenceParseError {
    /// The value is not canonical unsigned decimal text.
    #[error("session sequence must use canonical unsigned decimal text")]
    InvalidFormat,
    /// The value exceeds the supported integer range.
    #[error("session sequence is out of range")]
    OutOfRange,
}

impl fmt::Display for SessionSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SessionSequence {
    type Err = SessionSequenceParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(SessionSequenceParseError::InvalidFormat);
        }
        value
            .parse()
            .map(Self)
            .map_err(|_| SessionSequenceParseError::OutOfRange)
    }
}

impl Serialize for SessionSequence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SessionSequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}
