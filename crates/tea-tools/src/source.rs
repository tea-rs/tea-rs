use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

const MAX_SOURCE_ID_BYTES: usize = 256;
const SHA256_HEX_BYTES: usize = 64;
const NATIVE_PRODUCT_DIGEST: &str =
    "5c2fe6d5aa5a64dc09567ef61a02305036b9c96e3e2ad8fa6c4bd7eaeb234ecf";

/// Provider-neutral origin category for a registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSourceKind {
    /// Product-owned in-process or host-native implementation.
    Native,
    /// Tool exposed through the Model Context Protocol.
    Mcp,
    /// Tool executed by another remote adapter.
    Remote,
}

/// Host-assigned trust class for a tool source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTrust {
    /// Source shipped and controlled by the product.
    Product,
    /// Source configured directly by the current user.
    User,
    /// Source enabled by trusted workspace configuration.
    Workspace,
    /// Source has no affirmative trust assignment.
    Untrusted,
}

/// Stable, bounded provenance for one frozen tool descriptor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSource {
    kind: ToolSourceKind,
    source_id: String,
    trust: ToolTrust,
    descriptor_digest: String,
}

impl ToolSource {
    /// Creates validated tool-source provenance.
    ///
    /// # Errors
    ///
    /// Returns an error unless the source ID is canonical lowercase ASCII and
    /// the descriptor digest is exactly one lowercase hexadecimal SHA-256.
    pub fn new(
        kind: ToolSourceKind,
        source_id: impl Into<String>,
        trust: ToolTrust,
        descriptor_digest: impl Into<String>,
    ) -> Result<Self, ToolSourceError> {
        let source_id = source_id.into();
        let descriptor_digest = descriptor_digest.into();
        let mut bytes = source_id.bytes();
        if source_id.len() > MAX_SOURCE_ID_BYTES
            || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            })
        {
            return Err(ToolSourceError::InvalidSourceId);
        }
        if descriptor_digest.len() != SHA256_HEX_BYTES
            || !descriptor_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ToolSourceError::InvalidDescriptorDigest);
        }
        Ok(Self {
            kind,
            source_id,
            trust,
            descriptor_digest,
        })
    }

    /// Returns the stable default provenance used by native product tools.
    #[must_use]
    pub fn native_product() -> Self {
        Self {
            kind: ToolSourceKind::Native,
            source_id: "tea-rs.product.native".to_owned(),
            trust: ToolTrust::Product,
            descriptor_digest: NATIVE_PRODUCT_DIGEST.to_owned(),
        }
    }

    /// Returns whether this is the stable legacy-compatible product source.
    #[must_use]
    pub fn is_native_product(&self) -> bool {
        self.kind == ToolSourceKind::Native
            && self.source_id == "tea-rs.product.native"
            && self.trust == ToolTrust::Product
            && self.descriptor_digest == NATIVE_PRODUCT_DIGEST
    }

    /// Returns the provider-neutral source category.
    #[must_use]
    pub const fn kind(&self) -> ToolSourceKind {
        self.kind
    }

    /// Returns the stable canonical source identifier.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the host-assigned trust class.
    #[must_use]
    pub const fn trust(&self) -> ToolTrust {
        self.trust
    }

    /// Returns the lowercase SHA-256 descriptor digest.
    #[must_use]
    pub fn descriptor_digest(&self) -> &str {
        &self.descriptor_digest
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawToolSource {
    kind: ToolSourceKind,
    source_id: String,
    trust: ToolTrust,
    descriptor_digest: String,
}

impl<'de> Deserialize<'de> for ToolSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawToolSource::deserialize(deserializer)?;
        Self::new(raw.kind, raw.source_id, raw.trust, raw.descriptor_digest)
            .map_err(serde::de::Error::custom)
    }
}

/// Error returned when validating tool-source provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolSourceError {
    /// Source ID is empty, oversized, or not canonical lowercase ASCII.
    #[error("tool source ID is not canonical")]
    InvalidSourceId,
    /// Descriptor digest is not lowercase hexadecimal SHA-256 text.
    #[error("tool source descriptor digest is not lowercase SHA-256")]
    InvalidDescriptorDigest,
}
