use serde::{Deserialize, Deserializer, Serialize};
use serde_json::to_value;
use tea_protocol::{ProtocolMetadata, ProtocolMetadataError};
use thiserror::Error;

use crate::{ToolEffect, ToolResourceAccess, ToolSource, ToolVersion};

const MAX_AUDIT_EFFECTS: usize = 64;
const MAX_AUDIT_RESOURCES: usize = 128;
const MAX_RESOURCE_PRESENTATION_BYTES: usize = 2048;

/// Metadata namespace used on durable tool-call request envelopes.
pub const TOOL_AUDIT_METADATA_NAMESPACE: &str = "dev.tea-rs.tool-audit";

/// Already-redacted resource presentation safe for durable audit metadata.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAuditResource {
    scheme: String,
    redacted_presentation: String,
    access: ToolResourceAccess,
}

impl ToolAuditResource {
    /// Creates a bounded audit resource from an already-redacted presentation.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-canonical scheme or an empty, oversized, or
    /// control-containing presentation.
    pub fn new(
        scheme: impl Into<String>,
        redacted_presentation: impl Into<String>,
        access: ToolResourceAccess,
    ) -> Result<Self, ToolAuditMetadataError> {
        let scheme = scheme.into();
        let redacted_presentation = redacted_presentation.into();
        let mut bytes = scheme.bytes();
        if scheme.len() > 64
            || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ToolAuditMetadataError::InvalidResource);
        }
        if redacted_presentation.is_empty()
            || redacted_presentation.len() > MAX_RESOURCE_PRESENTATION_BYTES
            || redacted_presentation.chars().any(char::is_control)
        {
            return Err(ToolAuditMetadataError::InvalidResource);
        }
        Ok(Self {
            scheme,
            redacted_presentation,
            access,
        })
    }

    /// Returns the canonical resource scheme.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Returns the already-redacted resource presentation.
    #[must_use]
    pub fn redacted_presentation(&self) -> &str {
        &self.redacted_presentation
    }

    /// Returns requested resource access.
    #[must_use]
    pub const fn access(&self) -> ToolResourceAccess {
        self.access
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawToolAuditResource {
    scheme: String,
    redacted_presentation: String,
    access: ToolResourceAccess,
}

impl<'de> Deserialize<'de> for ToolAuditResource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawToolAuditResource::deserialize(deserializer)?;
        Self::new(raw.scheme, raw.redacted_presentation, raw.access)
            .map_err(serde::de::Error::custom)
    }
}

/// Bounded tool provenance and declared-capability metadata for durable audit.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAuditMetadata {
    tool_version: ToolVersion,
    source: ToolSource,
    effects: Vec<ToolEffect>,
    resources: Vec<ToolAuditResource>,
}

impl ToolAuditMetadata {
    /// Creates canonical audit metadata from already-validated tool context.
    ///
    /// # Errors
    ///
    /// Returns an error for missing or oversized effects, or too many resources.
    pub fn new(
        tool_version: ToolVersion,
        source: ToolSource,
        effects: impl IntoIterator<Item = ToolEffect>,
        resources: impl IntoIterator<Item = ToolAuditResource>,
    ) -> Result<Self, ToolAuditMetadataError> {
        let mut effects = effects.into_iter().collect::<Vec<_>>();
        effects.sort();
        effects.dedup();
        if effects.is_empty() || effects.len() > MAX_AUDIT_EFFECTS {
            return Err(ToolAuditMetadataError::InvalidEffects);
        }
        let mut resources = resources.into_iter().collect::<Vec<_>>();
        resources.sort();
        resources.dedup();
        if resources.len() > MAX_AUDIT_RESOURCES {
            return Err(ToolAuditMetadataError::InvalidResources);
        }
        Ok(Self {
            tool_version,
            source,
            effects,
            resources,
        })
    }

    /// Converts this value to one bounded, namespaced protocol metadata entry.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or protocol metadata bounds fail.
    pub fn to_protocol_metadata(&self) -> Result<ProtocolMetadata, ToolAuditMetadataError> {
        let value = to_value(self).map_err(ToolAuditMetadataError::Serialization)?;
        ProtocolMetadata::from_entries([(TOOL_AUDIT_METADATA_NAMESPACE, value)])
            .map_err(ToolAuditMetadataError::ProtocolMetadata)
    }

    /// Returns the frozen semantic tool version.
    #[must_use]
    pub const fn tool_version(&self) -> &ToolVersion {
        &self.tool_version
    }

    /// Returns the frozen tool-source provenance.
    #[must_use]
    pub const fn source(&self) -> &ToolSource {
        &self.source
    }

    /// Returns sorted, deduplicated effect names.
    #[must_use]
    pub fn effects(&self) -> &[ToolEffect] {
        &self.effects
    }

    /// Returns sorted, deduplicated redacted resources.
    #[must_use]
    pub fn resources(&self) -> &[ToolAuditResource] {
        &self.resources
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawToolAuditMetadata {
    tool_version: ToolVersion,
    source: ToolSource,
    effects: Vec<ToolEffect>,
    resources: Vec<ToolAuditResource>,
}

impl<'de> Deserialize<'de> for ToolAuditMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawToolAuditMetadata::deserialize(deserializer)?;
        Self::new(raw.tool_version, raw.source, raw.effects, raw.resources)
            .map_err(serde::de::Error::custom)
    }
}

/// Error returned when validating or encoding tool audit metadata.
#[derive(Debug, Error)]
pub enum ToolAuditMetadataError {
    /// Declared effects are empty or exceed the deterministic bound.
    #[error("tool audit effects are invalid")]
    InvalidEffects,
    /// Redacted resources exceed the deterministic bound.
    #[error("tool audit resources are invalid")]
    InvalidResources,
    /// One redacted resource is malformed or exceeds its text bound.
    #[error("tool audit resource is invalid")]
    InvalidResource,
    /// Audit serialization unexpectedly failed.
    #[error("tool audit metadata could not be encoded: {0}")]
    Serialization(serde_json::Error),
    /// Audit metadata exceeds protocol metadata constraints.
    #[error("tool audit metadata exceeds protocol bounds: {0}")]
    ProtocolMetadata(ProtocolMetadataError),
}
