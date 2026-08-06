use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use uuid::{Uuid, Version};

/// Error returned when parsing a canonical protocol identifier.
#[derive(Debug, Error)]
pub enum ProtocolIdParseError {
    /// The text is not a valid UUID.
    #[error("invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),
    /// The UUID is valid but is not canonical lowercase hyphenated text.
    #[error("protocol ID must use canonical lowercase hyphenated UUID text")]
    NonCanonical,
    /// The UUID does not use version 7.
    #[error("protocol ID must use UUID version 7")]
    WrongVersion,
}

fn parse_uuid_v7(value: &str) -> Result<Uuid, ProtocolIdParseError> {
    let uuid = Uuid::parse_str(value)?;
    if uuid.get_version() != Some(Version::SortRand) {
        return Err(ProtocolIdParseError::WrongVersion);
    }
    if uuid.hyphenated().to_string() != value {
        return Err(ProtocolIdParseError::NonCanonical);
    }
    Ok(uuid)
}

macro_rules! protocol_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Returns the underlying UUID value.
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = ProtocolIdParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_uuid_v7(value).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

protocol_id!(SessionId, "A stable agent session identifier.");
protocol_id!(RunId, "A stable agent run identifier.");
protocol_id!(TurnId, "A stable agent turn identifier.");
protocol_id!(MessageId, "A stable canonical message identifier.");
protocol_id!(ToolCallId, "A stable canonical tool-call identifier.");
protocol_id!(ApprovalId, "A stable approval request identifier.");
protocol_id!(EventId, "A stable observable event identifier.");
protocol_id!(CommandId, "A stable command identifier.");
protocol_id!(BranchId, "A stable session branch identifier.");
protocol_id!(RecordId, "A stable durable session record identifier.");
protocol_id!(CorrelationId, "A stable diagnostic correlation identifier.");
protocol_id!(
    CausationId,
    "A stable identifier for the command or record that caused a fact."
);
