use std::{fmt, fs, path::Path, str::FromStr, time::UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{McpError, McpErrorCode};

/// Maximum encoded bytes in one canonical MCP server identity.
pub const MAX_MCP_SERVER_ID_BYTES: usize = 64;
/// Maximum encoded bytes retained for one exact remote tool name.
pub const MAX_MCP_REMOTE_TOOL_NAME_BYTES: usize = 128;
const SHA256_HEX_BYTES: usize = 64;

/// Stable canonical identity for one explicitly configured MCP server.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct McpServerId(String);

impl McpServerId {
    /// Returns the canonical lowercase identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for McpServerId {
    type Err = McpError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !canonical_identifier(value, MAX_MCP_SERVER_ID_BYTES) {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for McpServerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for McpServerId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for McpServerId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Exact bounded tool name advertised by a remote MCP server.
///
/// Remote names are preserved without normalization. Names that cannot form an
/// existing `ToolName` require an explicit host alias before they can be enabled.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct McpRemoteToolName(String);

impl McpRemoteToolName {
    /// Creates an exact non-empty, control-free remote name.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, or control-containing names.
    pub fn new(value: impl Into<String>) -> Result<Self, McpError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_MCP_REMOTE_TOOL_NAME_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(McpError::new(McpErrorCode::Descriptor));
        }
        Ok(Self(value))
    }

    /// Returns the exact remote name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for McpRemoteToolName {
    type Err = McpError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for McpRemoteToolName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for McpRemoteToolName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for McpRemoteToolName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Canonical lowercase SHA-256 digest for a frozen MCP descriptor snapshot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct McpDescriptorDigest(String);

impl McpDescriptorDigest {
    /// Returns the lowercase hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for McpDescriptorDigest {
    type Err = McpError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != SHA256_HEX_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(McpError::new(McpErrorCode::Descriptor));
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for McpDescriptorDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for McpDescriptorDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for McpDescriptorDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

fn canonical_identifier(value: &str, maximum: usize) -> bool {
    let mut bytes = value.bytes();
    value.len() <= maximum
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

/// Private pre-spawn proof for an exact configured executable.
///
/// The proof never contains the configured path. Unix platforms add stable
/// device/inode data; all platforms retain bounded metadata as a best-effort
/// replacement detector before the OS executes the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct McpExecutableIdentity {
    length: u64,
    modified: Option<(u64, u32)>,
    #[cfg(unix)]
    unix: UnixExecutableIdentity,
}

impl McpExecutableIdentity {
    pub(crate) fn capture(executable: &Path) -> Result<Self, McpError> {
        let metadata =
            fs::metadata(executable).map_err(|_| McpError::new(McpErrorCode::Startup))?;
        if !metadata.is_file() {
            return Err(McpError::new(McpErrorCode::Startup));
        }
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified().ok().and_then(|modified| {
                modified
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .map(|duration| (duration.as_secs(), duration.subsec_nanos()))
            }),
            #[cfg(unix)]
            unix: UnixExecutableIdentity::from_metadata(&metadata),
        })
    }

    pub(crate) fn verify(&self, executable: &Path) -> Result<(), McpError> {
        if Self::capture(executable)? == *self {
            Ok(())
        } else {
            Err(McpError::new(McpErrorCode::Identity))
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnixExecutableIdentity {
    device: u64,
    inode: u64,
    changed: (i64, i64),
    mode: u32,
}

#[cfg(unix)]
impl UnixExecutableIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;

        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            mode: metadata.mode(),
        }
    }
}
