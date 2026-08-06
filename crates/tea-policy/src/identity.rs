use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tea_protocol::ProtocolMetadata;
use thiserror::Error;
use uuid::{Uuid, Version};

macro_rules! policy_selector {
    ($name:ident, $doc:literal, $allow_colon:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Returns canonical selector text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = PolicyIdentityParseError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                validate_selector(value, $allow_colon)?;
                Ok(Self(value.to_owned()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
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

policy_selector!(
    ActorId,
    "Bounded actor identity used for policy matching.",
    true
);
policy_selector!(
    WorkspaceId,
    "Bounded workspace identity used for policy matching.",
    false
);

/// Stable `UUIDv7` identity for a durable policy grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantId(Uuid);

impl FromStr for GrantId {
    type Err = PolicyIdentityParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parsed =
            Uuid::parse_str(value).map_err(|_| PolicyIdentityParseError::InvalidGrantId)?;
        if parsed.get_version() != Some(Version::SortRand)
            || parsed.hyphenated().to_string() != value
        {
            return Err(PolicyIdentityParseError::InvalidGrantId);
        }
        Ok(Self(parsed))
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for GrantId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for GrantId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Product surface requesting policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSurface {
    /// Desktop application.
    Desktop,
    /// Command-line host.
    Cli,
    /// Service/API host.
    Service,
    /// IDE integration.
    Ide,
    /// Deterministic test host.
    Test,
}

/// Execution location policy may select before a tool runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyExecutionTarget {
    /// Native in-process/OS adapter.
    Native,
    /// Owned subprocess adapter.
    Subprocess,
    /// Isolated sandbox adapter.
    Sandbox,
    /// Model Context Protocol adapter.
    Mcp,
    /// Remote execution adapter.
    Remote,
    /// WebAssembly adapter.
    Wasm,
}

/// Immutable execution environment visible to pure policy rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyEnvironment {
    surface: ExecutionSurface,
    target: PolicyExecutionTarget,
    metadata: ProtocolMetadata,
}

impl PolicyEnvironment {
    /// Creates an environment snapshot.
    #[must_use]
    pub const fn new(
        surface: ExecutionSurface,
        target: PolicyExecutionTarget,
        metadata: ProtocolMetadata,
    ) -> Self {
        Self {
            surface,
            target,
            metadata,
        }
    }
    /// Returns requesting product surface.
    #[must_use]
    pub const fn surface(&self) -> ExecutionSurface {
        self.surface
    }
    /// Returns current execution target.
    #[must_use]
    pub const fn target(&self) -> PolicyExecutionTarget {
        self.target
    }
    /// Returns bounded environment metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ProtocolMetadata {
        &self.metadata
    }
}

/// Error parsing policy identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PolicyIdentityParseError {
    /// Actor/workspace selector is not canonical.
    #[error("policy selector is not canonical")]
    InvalidSelector,
    /// Grant ID is not canonical `UUIDv7`.
    #[error("grant ID must be canonical UUIDv7")]
    InvalidGrantId,
}

fn validate_selector(value: &str, allow_colon: bool) -> Result<(), PolicyIdentityParseError> {
    let mut bytes = value.bytes();
    if value.len() > 128
        || value.contains("..")
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'@')
                || (allow_colon && byte == b':')
        })
    {
        Err(PolicyIdentityParseError::InvalidSelector)
    } else {
        Ok(())
    }
}
