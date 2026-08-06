use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Protocol version 1.0.
pub const PROTOCOL_V1_0: ProtocolVersion = ProtocolVersion::new(1, 0);

/// The protocol version written by this crate.
pub const CURRENT_PROTOCOL_VERSION: ProtocolVersion = PROTOCOL_V1_0;

/// A canonical `major.minor` agent protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    major: u16,
    minor: u16,
}

impl ProtocolVersion {
    /// Creates a protocol version from numeric major and minor components.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Returns the major version component.
    #[must_use]
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Returns the minor version component.
    #[must_use]
    pub const fn minor(self) -> u16 {
        self.minor
    }
}

/// Error returned when parsing a protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProtocolVersionParseError {
    /// The value is not canonical `major.minor` decimal text.
    #[error("protocol version must use canonical major.minor decimal text")]
    InvalidFormat,
    /// A component is outside the supported integer range.
    #[error("protocol version component is out of range")]
    ComponentOutOfRange,
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for ProtocolVersion {
    type Err = ProtocolVersionParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (major, minor) = value
            .split_once('.')
            .ok_or(ProtocolVersionParseError::InvalidFormat)?;
        if major.is_empty()
            || minor.is_empty()
            || minor.contains('.')
            || !canonical_decimal(major)
            || !canonical_decimal(minor)
        {
            return Err(ProtocolVersionParseError::InvalidFormat);
        }
        Ok(Self::new(
            major
                .parse()
                .map_err(|_| ProtocolVersionParseError::ComponentOutOfRange)?,
            minor
                .parse()
                .map_err(|_| ProtocolVersionParseError::ComponentOutOfRange)?,
        ))
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

fn canonical_decimal(value: &str) -> bool {
    value.bytes().all(|byte| byte.is_ascii_digit()) && (value == "0" || !value.starts_with('0'))
}
