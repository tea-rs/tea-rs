use serde_json::{Value, json};
use tea_tools::CompiledToolSchema;

use crate::{McpError, McpErrorCode};

pub(crate) fn validate_object_schema(schema: &Value) -> Result<(), McpError> {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(McpError::new(McpErrorCode::Schema));
    }
    CompiledToolSchema::compile(schema.clone())
        .map(|_| ())
        .map_err(|_| McpError::new(McpErrorCode::Schema))
}

pub(crate) fn default_output_schema() -> Value {
    json!({"type": "object"})
}
