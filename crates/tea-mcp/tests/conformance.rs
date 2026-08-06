#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use tea_control::CancellationScope;
use tea_mcp::McpLimits;
use tea_testkit::{ToolTerminalKind, collect_tool_execution};

use crate::support::Harness;

#[tokio::test]
async fn frozen_executor_is_lazy_bounded_and_conformant() {
    let harness = Harness::start("progress", McpLimits::default(), 2_000).await;
    let stream = harness.execute(CancellationScope::new());

    assert!(harness.marker_text().is_empty());
    let collected = collect_tool_execution(stream).await.unwrap();

    assert_eq!(
        collected.report().terminal_kind(),
        ToolTerminalKind::Finished
    );
    assert_eq!(collected.report().progress_count(), 2);
    assert_eq!(collected.report().event_count(), 3);
    assert_eq!(harness.marker_text(), "called\n");
    harness.shutdown().await;
}
