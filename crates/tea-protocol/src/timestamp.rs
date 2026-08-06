use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// An RFC 3339 timestamp normalized to UTC with millisecond precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolTimestamp(DateTime<Utc>);

impl ProtocolTimestamp {
    /// Returns the normalized UTC timestamp.
    #[must_use]
    pub const fn as_utc(self) -> DateTime<Utc> {
        self.0
    }
}

/// Error returned when parsing a protocol timestamp.
#[derive(Debug, Error)]
pub enum ProtocolTimestampParseError {
    /// The timestamp is not valid RFC 3339 text.
    #[error("timestamp is not valid RFC 3339: {0}")]
    InvalidRfc3339(#[from] chrono::ParseError),
    /// The timestamp does not have exactly three fractional second digits.
    #[error("timestamp must have exactly millisecond precision")]
    InvalidPrecision,
}

impl fmt::Display for ProtocolTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

impl FromStr for ProtocolTimestamp {
    type Err = ProtocolTimestampParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_millisecond_precision(value)?;
        Ok(Self(
            DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc),
        ))
    }
}

impl Serialize for ProtocolTimestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProtocolTimestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

fn validate_millisecond_precision(value: &str) -> Result<(), ProtocolTimestampParseError> {
    let time_start = value
        .find('T')
        .ok_or(ProtocolTimestampParseError::InvalidPrecision)?;
    let timezone_start = if value.ends_with('Z') {
        value.len() - 1
    } else {
        value[time_start + 1..]
            .rfind(['+', '-'])
            .map(|index| time_start + 1 + index)
            .ok_or(ProtocolTimestampParseError::InvalidPrecision)?
    };
    let fraction_start = value[time_start + 1..timezone_start]
        .find('.')
        .map(|index| time_start + 1 + index + 1)
        .ok_or(ProtocolTimestampParseError::InvalidPrecision)?;
    let fraction = &value[fraction_start..timezone_start];
    if fraction.len() != 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ProtocolTimestampParseError::InvalidPrecision);
    }
    Ok(())
}
