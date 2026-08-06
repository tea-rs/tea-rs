use std::str::FromStr;

use serde_json::{Value, json};
use tea_protocol::{ProtocolMetadata, ToolCallId};
use tea_tools::{
    MAX_TOOL_VALUE_BYTES, MAX_TOOL_VALUE_DEPTH, ToolInvocation, ToolInvocationError, ToolName,
};

fn invocation(arguments: Value) -> Result<ToolInvocation, ToolInvocationError> {
    ToolInvocation::new(
        ToolCallId::from_str("0195a0b1-7100-7000-8000-0aa7aa000001").unwrap(),
        ToolName::from_str("bounded_input").unwrap(),
        arguments,
        ProtocolMetadata::default(),
    )
}

fn nested_object(layers: usize) -> Value {
    let mut value = json!({});
    for _ in 0..layers {
        value = json!({"next": value});
    }
    value
}

#[test]
fn tool_arguments_reject_oversized_payloads_before_execution() {
    assert!(invocation(json!({"data": "x".repeat(MAX_TOOL_VALUE_BYTES / 2)})).is_ok());
    assert_eq!(
        invocation(json!({"data": "x".repeat(MAX_TOOL_VALUE_BYTES)})).unwrap_err(),
        ToolInvocationError::ArgumentsOutOfBounds
    );
}

#[test]
fn tool_arguments_reject_values_deeper_than_the_documented_limit() {
    assert!(invocation(nested_object(MAX_TOOL_VALUE_DEPTH - 1)).is_ok());
    assert_eq!(
        invocation(nested_object(MAX_TOOL_VALUE_DEPTH)).unwrap_err(),
        ToolInvocationError::ArgumentsOutOfBounds
    );
}

#[test]
fn generated_argument_boundaries_return_stable_results() {
    for layers in 0..=MAX_TOOL_VALUE_DEPTH + 8 {
        let result = invocation(nested_object(layers));
        if layers < MAX_TOOL_VALUE_DEPTH {
            assert!(result.is_ok(), "layers {layers}");
        } else {
            assert_eq!(
                result.unwrap_err(),
                ToolInvocationError::ArgumentsOutOfBounds,
                "layers {layers}"
            );
        }
    }
}
