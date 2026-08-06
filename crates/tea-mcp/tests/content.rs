#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use tea_control::CancellationScope;
use tea_mcp::McpLimits;
use tea_protocol::{ContentBlock, ImageSource};
use tea_testkit::{ToolTerminalKind, collect_tool_execution};
use tea_tools::{ToolExecutionEvent, ToolExecutionFailureCode};

use crate::support::Harness;

#[tokio::test]
async fn supported_mcp_content_maps_without_json_stringification() {
    let harness = Harness::start("content", McpLimits::default(), 2_000).await;
    let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
        .await
        .unwrap();
    let ToolExecutionEvent::Finished(result) = collected.events().last().unwrap() else {
        panic!("expected successful terminal result");
    };

    assert_eq!(result.output(), &serde_json::json!({"echo":"mapped"}));
    assert!(matches!(&result.content()[0], ContentBlock::Text { text } if text == "plain"));
    assert!(matches!(
        &result.content()[1],
        ContentBlock::Image {
            mime_type,
            source: ImageSource::InlineBase64 { data }
        } if mime_type == "image/png" && data == "aGVsbG8="
    ));
    assert!(matches!(&result.content()[2], ContentBlock::Text { text } if text == "embedded"));
    assert!(matches!(&result.content()[3], ContentBlock::Image { .. }));
    harness.shutdown().await;
}

#[tokio::test]
async fn structured_only_results_get_fixed_model_text() {
    let harness = Harness::start("structured-only", McpLimits::default(), 2_000).await;
    let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
        .await
        .unwrap();
    let ToolExecutionEvent::Finished(result) = collected.events().last().unwrap() else {
        panic!("expected successful terminal result");
    };

    assert_eq!(result.output(), &serde_json::json!({"echo":"structured"}));
    assert!(
        matches!(&result.content()[0], ContentBlock::Text { text } if text == "MCP tool returned structured output")
    );
    harness.shutdown().await;
}

#[tokio::test]
async fn unsupported_content_fails_closed() {
    let harness = Harness::start("unsupported", McpLimits::default(), 2_000).await;
    let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
        .await
        .unwrap();

    assert_eq!(collected.report().terminal_kind(), ToolTerminalKind::Failed);
    assert_eq!(
        collected.report().failure_code(),
        Some(ToolExecutionFailureCode::InvalidOutput)
    );
    harness.shutdown().await;
}
