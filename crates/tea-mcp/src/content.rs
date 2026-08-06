use rmcp::model::{CallToolResult, ContentBlock as McpContentBlock, ResourceContents};
use serde_json::json;
use tea_protocol::ContentBlock;
use tea_tools::{CompiledToolSchema, ToolResult};

use crate::{McpError, McpErrorCode};

const STRUCTURED_ONLY_TEXT: &str = "MCP tool returned structured output";
const EMPTY_SUCCESS_TEXT: &str = "MCP tool completed successfully";

pub(crate) enum MappedCallResult {
    Success(Box<ToolResult>),
    RemoteError,
}

pub(crate) fn map_call_result(
    result: CallToolResult,
    maximum_bytes: usize,
    output_schema: &CompiledToolSchema,
) -> Result<MappedCallResult, McpError> {
    let encoded =
        serde_json::to_vec(&result).map_err(|_| McpError::new(McpErrorCode::InvalidResult))?;
    if encoded.len() > maximum_bytes {
        return Err(McpError::new(McpErrorCode::OutputBound));
    }
    if result.is_error.unwrap_or(false) {
        return Ok(MappedCallResult::RemoteError);
    }

    let has_structured_output = result.structured_content.is_some();
    let output = result.structured_content.unwrap_or_else(|| json!({}));
    if !output.is_object() {
        return Err(McpError::new(McpErrorCode::InvalidResult));
    }
    output_schema
        .validate(&output)
        .map_err(|_| McpError::new(McpErrorCode::Schema))?;

    let mut content = result
        .content
        .into_iter()
        .map(map_content_block)
        .collect::<Result<Vec<_>, _>>()?;
    if content.is_empty() {
        let text = if has_structured_output {
            STRUCTURED_ONLY_TEXT
        } else {
            EMPTY_SUCCESS_TEXT
        };
        content.push(
            ContentBlock::text(text).map_err(|_| McpError::new(McpErrorCode::InvalidResult))?,
        );
    }
    ToolResult::new(content, output)
        .map(Box::new)
        .map(MappedCallResult::Success)
        .map_err(|_| McpError::new(McpErrorCode::InvalidResult))
}

fn map_content_block(block: McpContentBlock) -> Result<ContentBlock, McpError> {
    match block {
        McpContentBlock::Text(text) => {
            ContentBlock::text(text.text).map_err(|_| McpError::new(McpErrorCode::InvalidResult))
        }
        McpContentBlock::Image(image) => ContentBlock::inline_image(image.mime_type, image.data)
            .map_err(|_| McpError::new(McpErrorCode::InvalidResult)),
        McpContentBlock::Resource(resource) => map_resource(resource.resource),
        McpContentBlock::Audio(_) | McpContentBlock::ResourceLink(_) => {
            Err(McpError::new(McpErrorCode::Protocol))
        }
        _ => Err(McpError::new(McpErrorCode::Protocol)),
    }
}

fn map_resource(resource: ResourceContents) -> Result<ContentBlock, McpError> {
    match resource {
        ResourceContents::TextResourceContents { text, .. } => {
            ContentBlock::text(text).map_err(|_| McpError::new(McpErrorCode::InvalidResult))
        }
        ResourceContents::BlobResourceContents {
            mime_type, blob, ..
        } => ContentBlock::inline_image(
            mime_type.ok_or_else(|| McpError::new(McpErrorCode::Protocol))?,
            blob,
        )
        .map_err(|_| McpError::new(McpErrorCode::InvalidResult)),
        _ => Err(McpError::new(McpErrorCode::Protocol)),
    }
}
