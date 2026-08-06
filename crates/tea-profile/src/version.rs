use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Profile schema version 1.0.0.
pub const PROFILE_SCHEMA_V1_0_0: ProfileSchemaVersion = ProfileSchemaVersion {
    major: 1,
    minor: 0,
    patch: 0,
};

/// The profile schema version written by this crate.
pub const CURRENT_PROFILE_SCHEMA_VERSION: ProfileSchemaVersion = PROFILE_SCHEMA_V1_0_0;

/// A canonical `major.minor.patch` product profile schema version.
///
/// The schema version is independent of protocol compatibility and crate
/// `SemVer`. It is checked at every deserialization and composition so profiles
/// with divergent schema versions cannot bind or compose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileSchemaVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

impl ProfileSchemaVersion {
    /// Creates a schema version from numeric components.
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Returns the schema version supported by this crate.
    #[must_use]
    pub const fn current() -> Self {
        CURRENT_PROFILE_SCHEMA_VERSION
    }

    /// Returns whether this crate can read and bind this schema version.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        self.major == CURRENT_PROFILE_SCHEMA_VERSION.major
    }

    /// Returns the major component.
    #[must_use]
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Returns the minor component.
    #[must_use]
    pub const fn minor(self) -> u16 {
        self.minor
    }

    /// Returns the patch component.
    #[must_use]
    pub const fn patch(self) -> u16 {
        self.patch
    }
}

/// Error returned when parsing a profile schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProfileSchemaVersionParseError {
    /// The value is not canonical `major.minor.patch` decimal text.
    #[error("profile schema version must use canonical major.minor.patch decimal text")]
    InvalidFormat,
    /// A component is outside the supported integer range.
    #[error("profile schema version component is out of range")]
    ComponentOutOfRange,
}

impl fmt::Display for ProfileSchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for ProfileSchemaVersion {
    type Err = ProfileSchemaVersionParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = value.split('.').collect();
        if parts.len() != 3 {
            return Err(ProfileSchemaVersionParseError::InvalidFormat);
        }
        let [major, minor, patch] = [parts[0], parts[1], parts[2]];
        if !canonical_decimal(major) || !canonical_decimal(minor) || !canonical_decimal(patch) {
            return Err(ProfileSchemaVersionParseError::InvalidFormat);
        }
        Ok(Self::new(
            major
                .parse()
                .map_err(|_| ProfileSchemaVersionParseError::ComponentOutOfRange)?,
            minor
                .parse()
                .map_err(|_| ProfileSchemaVersionParseError::ComponentOutOfRange)?,
            patch
                .parse()
                .map_err(|_| ProfileSchemaVersionParseError::ComponentOutOfRange)?,
        ))
    }
}

impl Serialize for ProfileSchemaVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProfileSchemaVersion {
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
