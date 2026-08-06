use std::{fmt, str::FromStr};

use rmcp::model::{TaskSupport, Tool};
use serde_json::{Map, Value};

use crate::{MAX_MCP_DESCRIPTOR_BYTES, McpError, McpErrorCode, McpRemoteToolName, schema};

const MAX_DESCRIPTOR_DEPTH: usize = 32;
const MAX_DESCRIPTION_BYTES: usize = 16 * 1024;

/// Bounded, SDK-independent snapshot of one remote MCP tool descriptor.
#[derive(Clone, PartialEq)]
pub struct McpRemoteToolDescriptor {
    name: McpRemoteToolName,
    description: String,
    input_schema: Value,
    output_schema: Value,
    annotations: Option<Value>,
    canonical_json: Vec<u8>,
}

impl McpRemoteToolDescriptor {
    /// Parses and validates one MCP tool descriptor represented as JSON.
    ///
    /// The returned value owns canonical, bounded JSON and contains no public
    /// MCP SDK type. Both schemas are compiled offline before this succeeds.
    ///
    /// # Errors
    ///
    /// Returns a stable descriptor, schema, or output-bound classification for
    /// malformed text, unsupported task requirements, invalid schemas, or JSON
    /// outside the hard descriptor bounds.
    pub fn from_value(value: Value) -> Result<Self, McpError> {
        validate_json_bounds(&value, MAX_MCP_DESCRIPTOR_BYTES)?;
        let tool =
            serde_json::from_value(value).map_err(|_| McpError::new(McpErrorCode::Descriptor))?;
        Self::from_sdk_tool(&tool, MAX_MCP_DESCRIPTOR_BYTES)
    }

    /// Returns the exact remote tool name without normalization.
    #[must_use]
    pub const fn name(&self) -> &McpRemoteToolName {
        &self.name
    }

    /// Returns the validated model-visible description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the validated input object schema.
    #[must_use]
    pub const fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// Returns the remote output schema or the adapter's default object schema.
    #[must_use]
    pub const fn output_schema(&self) -> &Value {
        &self.output_schema
    }

    /// Returns bounded remote annotations retained only as untrusted diagnostics.
    #[must_use]
    pub const fn annotations(&self) -> Option<&Value> {
        self.annotations.as_ref()
    }

    pub(crate) fn from_sdk_tool(tool: &Tool, maximum_bytes: usize) -> Result<Self, McpError> {
        if tool
            .execution
            .as_ref()
            .and_then(|execution| execution.task_support)
            .is_some_and(|support| support == TaskSupport::Required)
        {
            return Err(McpError::new(McpErrorCode::Descriptor));
        }

        let name = McpRemoteToolName::new(tool.name.as_ref())?;
        let description = tool
            .description
            .as_deref()
            .filter(|description| {
                !description.is_empty()
                    && description.len() <= MAX_DESCRIPTION_BYTES
                    && !description.contains('\0')
            })
            .ok_or_else(|| McpError::new(McpErrorCode::Descriptor))?
            .to_owned();
        let input_schema = canonicalize(Value::Object((*tool.input_schema).clone()), 1)?;
        let output_schema = tool.output_schema.as_ref().map_or_else(
            || Ok(schema::default_output_schema()),
            |schema| canonicalize(Value::Object((**schema).clone()), 1),
        )?;
        schema::validate_object_schema(&input_schema)?;
        schema::validate_object_schema(&output_schema)?;

        let canonical = canonicalize(
            serde_json::to_value(tool).map_err(|_| McpError::new(McpErrorCode::Descriptor))?,
            1,
        )?;
        let canonical_json =
            serde_json::to_vec(&canonical).map_err(|_| McpError::new(McpErrorCode::Descriptor))?;
        if canonical_json.len() > maximum_bytes {
            return Err(McpError::new(McpErrorCode::OutputBound));
        }
        let annotations = canonical.get("annotations").cloned();
        Ok(Self {
            name,
            description,
            input_schema,
            output_schema,
            annotations,
            canonical_json,
        })
    }

    pub(crate) fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }
}

impl fmt::Debug for McpRemoteToolDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpRemoteToolDescriptor")
            .field("name", &self.name)
            .field("descriptor_bytes", &self.canonical_json.len())
            .field(
                "annotations",
                &self.annotations.as_ref().map(|_| "<untrusted>"),
            )
            .finish_non_exhaustive()
    }
}

pub(crate) fn canonical_json(value: Value, maximum_bytes: usize) -> Result<Vec<u8>, McpError> {
    validate_json_bounds(&value, maximum_bytes)?;
    let canonical = canonicalize(value, 1)?;
    let bytes =
        serde_json::to_vec(&canonical).map_err(|_| McpError::new(McpErrorCode::Descriptor))?;
    if bytes.len() > maximum_bytes {
        return Err(McpError::new(McpErrorCode::OutputBound));
    }
    Ok(bytes)
}

fn canonicalize(value: Value, depth: usize) -> Result<Value, McpError> {
    if depth > MAX_DESCRIPTOR_DEPTH {
        return Err(McpError::new(McpErrorCode::OutputBound));
    }
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(|value| canonicalize(value, depth.saturating_add(1)))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize(value, depth.saturating_add(1))?);
            }
            Ok(Value::Object(canonical))
        }
        scalar => Ok(scalar),
    }
}

fn validate_json_bounds(value: &Value, maximum_bytes: usize) -> Result<(), McpError> {
    if json_depth(value, 1) > MAX_DESCRIPTOR_DEPTH {
        return Err(McpError::new(McpErrorCode::OutputBound));
    }
    let bytes = serde_json::to_vec(value).map_err(|_| McpError::new(McpErrorCode::Descriptor))?;
    if bytes.len() > maximum_bytes {
        return Err(McpError::new(McpErrorCode::OutputBound));
    }
    Ok(())
}

fn json_depth(value: &Value, depth: usize) -> usize {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| json_depth(value, depth.saturating_add(1)))
            .max()
            .unwrap_or(depth),
        Value::Object(values) => values
            .values()
            .map(|value| json_depth(value, depth.saturating_add(1)))
            .max()
            .unwrap_or(depth),
        _ => depth,
    }
}

pub(crate) fn descriptor_digest(value: &str) -> Result<crate::McpDescriptorDigest, McpError> {
    crate::McpDescriptorDigest::from_str(value)
}
