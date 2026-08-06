use serde_json::Value;
use tea_protocol::{ContentBlock, ProtocolMetadata};
use tea_tools::{ToolExecutionEvent, ToolExecutionFailure, ToolResult};

use crate::{FileToolError, FileToolErrorCode};

pub(crate) fn success(text: String, output: Value) -> ToolExecutionEvent {
    let result = ContentBlock::text(text)
        .map_err(|_| FileToolError::new(FileToolErrorCode::Internal))
        .and_then(|content| {
            ToolResult::new(vec![content], output)
                .map_err(|_| FileToolError::new(FileToolErrorCode::Internal))
        });
    match result {
        Ok(result) => ToolExecutionEvent::Finished(result),
        Err(error) => failure(error),
    }
}

pub(crate) fn failure(error: FileToolError) -> ToolExecutionEvent {
    let details = ProtocolMetadata::from_entries([(
        "dev.tea-rs.coding-tools",
        serde_json::json!({"code":error.code().as_str()}),
    )])
    .unwrap_or_default();
    ToolExecutionEvent::Failed(
        ToolExecutionFailure::execution(error.message())
            .unwrap_or_else(|_| ToolExecutionFailure::internal_contract())
            .with_details(details),
    )
}
