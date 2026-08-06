use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

const MAX_REDACTED_BYTES: usize = 64 * 1024;
const MAX_REDACTED_DEPTH: usize = 32;
const MAX_RESOURCE_PRESENTATION_BYTES: usize = 2048;
const REDACTED: &str = "[REDACTED]";

/// Bounded redacted JSON presentation safe for approval surfaces.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RedactedArguments(Value);

impl RedactedArguments {
    /// Returns structure-preserving redacted JSON.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RedactedArguments {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        if serde_json::to_vec(&value)
            .map_err(serde::de::Error::custom)?
            .len()
            > MAX_REDACTED_BYTES
            || json_depth(&value) > MAX_REDACTED_DEPTH
        {
            return Err(serde::de::Error::custom(
                "redacted arguments exceed supported bounds",
            ));
        }
        Ok(Self(value))
    }
}

/// Deterministic explicit-key policy redactor.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolicyRedactor;

impl PolicyRedactor {
    /// Redacts nested sensitive values while preserving object/array structure.
    ///
    /// # Errors
    ///
    /// Returns an error when the redacted result exceeds byte/depth bounds.
    pub fn redact_arguments(&self, value: &Value) -> Result<RedactedArguments, RedactionError> {
        let redacted = redact_value(value);
        if serde_json::to_vec(&redacted)
            .map_err(|_| RedactionError::OutputOutOfBounds)?
            .len()
            > MAX_REDACTED_BYTES
            || json_depth(&redacted) > MAX_REDACTED_DEPTH
        {
            return Err(RedactionError::OutputOutOfBounds);
        }
        Ok(RedactedArguments(redacted))
    }

    /// Creates a bounded resource presentation with secret/query redaction.
    #[must_use]
    pub fn redact_resource(&self, scheme: &str, locator: &str) -> String {
        let value = if matches!(scheme, "credential" | "secret") {
            REDACTED.to_owned()
        } else if let Some((base, _)) = locator.split_once('?') {
            format!("{base}?[REDACTED]")
        } else {
            locator.to_owned()
        };
        truncate_utf8(
            &format!("{scheme}:{value}"),
            MAX_RESOURCE_PRESENTATION_BYTES,
        )
    }
}

/// Redaction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RedactionError {
    /// Redacted output exceeds encoded byte or nesting limits.
    #[error("redacted approval presentation exceeds supported bounds")]
    OutputOutOfBounds,
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(redact_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(key) {
                        Value::String(REDACTED.to_owned())
                    } else {
                        redact_value(value)
                    };
                    (key.clone(), value)
                })
                .collect::<Map<_, _>>(),
        ),
        _ => value.clone(),
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "token"
            | "accesstoken"
            | "refreshtoken"
            | "password"
            | "secret"
            | "clientsecret"
            | "apikey"
            | "authorization"
            | "cookie"
            | "privatekey"
    )
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
