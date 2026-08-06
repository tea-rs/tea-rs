use std::{collections::BTreeMap, fmt, fmt::Write as _, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tea_tools::ToolName;

use crate::{
    MAX_MCP_DESCRIPTOR_BYTES, MAX_MCP_TOOLS_PER_SERVER, McpDescriptorDigest, McpError,
    McpErrorCode, McpServerId, McpToolCatalog, descriptor,
};

/// Maximum bytes retained for one exact negotiated MCP protocol version.
pub const MAX_MCP_PROTOCOL_VERSION_BYTES: usize = 64;

/// Exact bounded MCP protocol version negotiated with one server.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct McpProtocolVersion(String);

impl McpProtocolVersion {
    /// Returns the exact negotiated version text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for McpProtocolVersion {
    type Err = McpError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > MAX_MCP_PROTOCOL_VERSION_BYTES
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(McpError::new(McpErrorCode::Descriptor));
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for McpProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for McpProtocolVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for McpProtocolVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Frozen secret-free identity and catalog proof for one initialized server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSnapshot {
    server_id: McpServerId,
    implementation_digest: McpDescriptorDigest,
    protocol_version: McpProtocolVersion,
    catalog_digest: McpDescriptorDigest,
    binding_digests: BTreeMap<ToolName, McpDescriptorDigest>,
}

impl McpServerSnapshot {
    /// Creates a snapshot and derives its catalog digest from exact bindings.
    ///
    /// # Errors
    ///
    /// Rejects duplicate aliases or a binding map above the per-server bound.
    pub fn new(
        server_id: McpServerId,
        implementation_digest: McpDescriptorDigest,
        protocol_version: McpProtocolVersion,
        bindings: impl IntoIterator<Item = (ToolName, McpDescriptorDigest)>,
    ) -> Result<Self, McpError> {
        let mut binding_digests = BTreeMap::new();
        for (alias, digest) in bindings {
            if binding_digests.len() >= MAX_MCP_TOOLS_PER_SERVER
                || binding_digests.insert(alias, digest).is_some()
            {
                return Err(McpError::new(McpErrorCode::Descriptor));
            }
        }
        let catalog_digest = catalog_digest(&binding_digests)?;
        Ok(Self {
            server_id,
            implementation_digest,
            protocol_version,
            catalog_digest,
            binding_digests,
        })
    }

    pub(crate) fn freeze(
        server_id: McpServerId,
        handshake: McpHandshakeSnapshot,
        catalog: &McpToolCatalog,
    ) -> Result<Self, McpError> {
        let bindings = catalog
            .bindings()
            .map(|binding| {
                binding
                    .spec()
                    .source()
                    .descriptor_digest()
                    .parse()
                    .map(|digest| (binding.spec().name().clone(), digest))
            })
            .collect::<Result<Vec<_>, McpError>>()?;
        Self::new(
            server_id,
            handshake.implementation_digest,
            handshake.protocol_version,
            bindings,
        )
    }

    /// Returns the configured server ID.
    #[must_use]
    pub const fn server_id(&self) -> &McpServerId {
        &self.server_id
    }

    /// Returns the hash of the complete server implementation descriptor.
    #[must_use]
    pub const fn implementation_digest(&self) -> &McpDescriptorDigest {
        &self.implementation_digest
    }

    /// Returns the exact negotiated protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> &McpProtocolVersion {
        &self.protocol_version
    }

    /// Returns the deterministic digest of the alias-to-binding map.
    #[must_use]
    pub const fn catalog_digest(&self) -> &McpDescriptorDigest {
        &self.catalog_digest
    }

    /// Returns binding digests in canonical alias order.
    pub fn binding_digests(&self) -> impl Iterator<Item = (&ToolName, &McpDescriptorDigest)> {
        self.binding_digests.iter()
    }

    /// Returns the frozen descriptor digest for one canonical alias.
    #[must_use]
    pub fn binding_digest(&self, alias: &ToolName) -> Option<&str> {
        self.binding_digests
            .get(alias)
            .map(McpDescriptorDigest::as_str)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawMcpServerSnapshot {
    server_id: McpServerId,
    implementation_digest: McpDescriptorDigest,
    protocol_version: McpProtocolVersion,
    catalog_digest: McpDescriptorDigest,
    binding_digests: BTreeMap<ToolName, McpDescriptorDigest>,
}

impl<'de> Deserialize<'de> for McpServerSnapshot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawMcpServerSnapshot::deserialize(deserializer)?;
        let expected_catalog_digest = raw.catalog_digest;
        let snapshot = Self::new(
            raw.server_id,
            raw.implementation_digest,
            raw.protocol_version,
            raw.binding_digests,
        )
        .map_err(serde::de::Error::custom)?;
        if snapshot.catalog_digest != expected_catalog_digest {
            return Err(serde::de::Error::custom("MCP catalog digest mismatch"));
        }
        Ok(snapshot)
    }
}

pub(crate) struct McpHandshakeSnapshot {
    implementation_digest: McpDescriptorDigest,
    protocol_version: McpProtocolVersion,
}

impl McpHandshakeSnapshot {
    pub(crate) fn freeze(protocol_version: &str, implementation: Value) -> Result<Self, McpError> {
        Ok(Self {
            implementation_digest: digest_value(b"tea-mcp-implementation-v1\0", implementation)?,
            protocol_version: protocol_version.parse()?,
        })
    }
}

fn catalog_digest(
    bindings: &BTreeMap<ToolName, McpDescriptorDigest>,
) -> Result<McpDescriptorDigest, McpError> {
    let mut map = Map::new();
    for (alias, digest) in bindings {
        map.insert(
            alias.as_str().to_owned(),
            Value::String(digest.as_str().to_owned()),
        );
    }
    digest_value(b"tea-mcp-catalog-v1\0", Value::Object(map))
}

fn digest_value(namespace: &[u8], value: Value) -> Result<McpDescriptorDigest, McpError> {
    let canonical = descriptor::canonical_json(value, MAX_MCP_DESCRIPTOR_BYTES)?;
    let length =
        u64::try_from(canonical.len()).map_err(|_| McpError::new(McpErrorCode::OutputBound))?;
    let mut hasher = Sha256::new();
    hasher.update(namespace);
    hasher.update(length.to_be_bytes());
    hasher.update(canonical);
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut digest, "{byte:02x}").map_err(|_| McpError::new(McpErrorCode::Descriptor))?;
    }
    descriptor::descriptor_digest(&digest)
}
