use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// Maximum UTF-8 bytes in a profile display name.
pub const MAX_PROFILE_DISPLAY_NAME_BYTES: usize = 256;
/// Maximum UTF-8 bytes in a profile description.
pub const MAX_PROFILE_DESCRIPTION_BYTES: usize = 4_096;

macro_rules! bounded_text {
    ($name:ident, $doc:literal, $max:ident) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        pub struct $name(String);

        impl $name {
            /// Creates a bounded non-empty text value.
            ///
            /// # Errors
            ///
            /// Returns an error for empty, oversized, or null-containing text.
            pub fn new(value: impl Into<String>) -> Result<Self, ProfileTextError> {
                let value = value.into();
                if value.is_empty() || value.len() > $max || value.contains('\0') {
                    return Err(ProfileTextError);
                }
                Ok(Self(value))
            }

            /// Returns canonical text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = ProfileTextError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_text!(
    ProfileDisplayName,
    "Bounded non-empty product profile display name.",
    MAX_PROFILE_DISPLAY_NAME_BYTES
);
bounded_text!(
    ProfileDescription,
    "Bounded non-empty product profile description.",
    MAX_PROFILE_DESCRIPTION_BYTES
);

/// Error returned when bounded profile text is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("profile text must be non-empty, bounded, and null-free")]
pub struct ProfileTextError;

/// Maximum UTF-8 bytes in a profile segment identity.
pub const MAX_PROFILE_SEGMENT_ID_BYTES: usize = 128;

/// Canonical bounded profile segment identity matching context segment ids.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProfileSegmentId(String);

impl ProfileSegmentId {
    /// Returns canonical selector text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProfileSegmentId {
    type Err = ProfileSelectorError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_id(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for ProfileSegmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProfileSegmentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Canonical `layer.id`-shaped reference to a registered policy rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProfileRuleId(String);

impl ProfileRuleId {
    /// Returns canonical rule reference text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProfileRuleId {
    type Err = ProfileSelectorError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_rule_id(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for ProfileRuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProfileRuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Origin trust label for profile-declared prompt content.
///
/// This mirrors the context crate's `TrustLevel` so the profile schema does
/// not depend on the context crate; the runtime converts the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileTrustLevel {
    /// Content is owned by the runtime/product trust boundary.
    Trusted,
    /// Content is supplied by a configured delegated source.
    Delegated,
    /// Content is caller-marked untrusted and receives no safety claim.
    Untrusted,
}

/// Invalid canonical profile selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("profile selector is not canonical")]
pub struct ProfileSelectorError;

fn validate_id(value: &str) -> Result<(), ProfileSelectorError> {
    let mut bytes = value.bytes();
    if value.len() > MAX_PROFILE_SEGMENT_ID_BYTES
        || value.contains("..")
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b'/')
        })
    {
        Err(ProfileSelectorError)
    } else {
        Ok(())
    }
}

fn validate_rule_id(value: &str) -> Result<(), ProfileSelectorError> {
    let mut bytes = value.bytes();
    if value.len() > MAX_PROFILE_SEGMENT_ID_BYTES
        || value.contains("..")
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
        || !value.contains('.')
    {
        Err(ProfileSelectorError)
    } else {
        Ok(())
    }
}
