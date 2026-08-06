use std::collections::BTreeMap;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::envelope::deserialize_unique_value;

/// Maximum number of metadata namespaces on one protocol value.
pub const MAX_METADATA_NAMESPACES: usize = 16;
/// Maximum encoded JSON bytes for one metadata map.
pub const MAX_METADATA_BYTES: usize = 16 * 1024;
/// Maximum JSON container nesting depth inside metadata values.
pub const MAX_METADATA_DEPTH: usize = 8;

/// Bounded extension metadata keyed by collision-resistant namespaces.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(transparent)]
pub struct ProtocolMetadata(BTreeMap<String, Value>);

impl ProtocolMetadata {
    pub(crate) fn protocol_version_details(received_version: &str) -> Self {
        Self(BTreeMap::from([(
            "dev.tea-rs.protocol".to_owned(),
            json!({
                "supportedProtocol": ">=1.0 <2.0",
                "receivedProtocol": received_version,
            }),
        )]))
    }

    pub(crate) fn protocol_compatibility_details(unsupported_type: Option<&str>) -> Self {
        let value = match unsupported_type {
            Some(unsupported_type) => json!({
                "supportedProtocol": ">=1.0 <2.0",
                "unsupportedType": unsupported_type,
            }),
            None => json!({"supportedProtocol": ">=1.0 <2.0"}),
        };
        Self(BTreeMap::from([("dev.tea-rs.protocol".to_owned(), value)]))
    }

    /// Builds and validates metadata from namespace/value entries.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid namespace or when count, byte, or
    /// nesting limits are exceeded.
    pub fn from_entries<K, I>(entries: I) -> Result<Self, ProtocolMetadataError>
    where
        K: Into<String>,
        I: IntoIterator<Item = (K, Value)>,
    {
        Self::try_from(
            entries
                .into_iter()
                .map(|(namespace, value)| (namespace.into(), value))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    /// Returns the number of namespaces.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the metadata map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns a metadata value by namespace.
    #[must_use]
    pub fn get(&self, namespace: &str) -> Option<&Value> {
        self.0.get(namespace)
    }
}

impl Deref for ProtocolMetadata {
    type Target = BTreeMap<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<BTreeMap<String, Value>> for ProtocolMetadata {
    type Error = ProtocolMetadataError;

    fn try_from(values: BTreeMap<String, Value>) -> Result<Self, Self::Error> {
        if values.len() > MAX_METADATA_NAMESPACES {
            return Err(ProtocolMetadataError::TooManyNamespaces);
        }
        for (namespace, value) in &values {
            validate_namespace(namespace)?;
            if json_depth(value) > MAX_METADATA_DEPTH {
                return Err(ProtocolMetadataError::TooDeep);
            }
        }
        if serde_json::to_vec(&values)
            .map_err(ProtocolMetadataError::Serialization)?
            .len()
            > MAX_METADATA_BYTES
        {
            return Err(ProtocolMetadataError::TooLarge);
        }
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for ProtocolMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_unique_value(deserializer)?;
        let values = serde_json::from_value::<BTreeMap<String, Value>>(value)
            .map_err(serde::de::Error::custom)?;
        Self::try_from(values).map_err(serde::de::Error::custom)
    }
}

/// Error returned when validating extension metadata.
#[derive(Debug, Error)]
pub enum ProtocolMetadataError {
    /// A namespace does not follow the required reverse-domain form.
    #[error("metadata namespace must contain at least two lowercase domain-style labels")]
    InvalidNamespace,
    /// The map contains too many namespaces.
    #[error("metadata contains too many namespaces")]
    TooManyNamespaces,
    /// The encoded map exceeds the byte limit.
    #[error("metadata exceeds the encoded byte limit")]
    TooLarge,
    /// A value exceeds the nesting-depth limit.
    #[error("metadata exceeds the nesting-depth limit")]
    TooDeep,
    /// JSON size validation unexpectedly failed.
    #[error("metadata could not be encoded for validation: {0}")]
    Serialization(serde_json::Error),
}

pub(crate) fn validate_json_bounds(
    value: &Value,
    max_bytes: usize,
    max_depth: usize,
) -> Result<(), ProtocolMetadataError> {
    if json_depth(value) > max_depth {
        return Err(ProtocolMetadataError::TooDeep);
    }
    if serde_json::to_vec(value)
        .map_err(ProtocolMetadataError::Serialization)?
        .len()
        > max_bytes
    {
        return Err(ProtocolMetadataError::TooLarge);
    }
    Ok(())
}

fn validate_namespace(namespace: &str) -> Result<(), ProtocolMetadataError> {
    let labels = namespace.split('.').collect::<Vec<_>>();
    if labels.len() < 2 || labels.iter().any(|label| !valid_label(label)) {
        return Err(ProtocolMetadataError::InvalidNamespace);
    }
    Ok(())
}

fn valid_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}
