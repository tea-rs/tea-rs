use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

macro_rules! context_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Returns canonical selector text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = ContextIdentityError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                validate_id(value)?;
                Ok(Self(value.to_owned()))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

context_id!(PromptModuleId, "Canonical prompt-module identity.");
context_id!(PromptSegmentId, "Canonical prompt-segment identity.");
context_id!(ContextProviderId, "Canonical context-provider identity.");
context_id!(
    ConflictKey,
    "Canonical key for mutually exclusive prompt claims."
);
context_id!(SkillId, "Canonical skill metadata identity.");

/// Invalid canonical context selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("context identifier is not canonical")]
pub struct ContextIdentityError;

fn validate_id(value: &str) -> Result<(), ContextIdentityError> {
    let mut bytes = value.bytes();
    if value.len() > 128
        || value.contains("..")
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b'/')
        })
    {
        Err(ContextIdentityError)
    } else {
        Ok(())
    }
}
