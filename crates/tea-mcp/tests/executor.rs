#![cfg(feature = "fixture-server")]
#![forbid(unsafe_code)]

use std::time::Duration;

use futures_util::StreamExt;
use tea_control::CancellationScope;
use tea_mcp::{McpErrorCode, McpLimits};
use tea_testkit::{ToolTerminalKind, collect_tool_execution};
use tea_tools::{ToolExecutionEvent, ToolExecutionFailureCode};

use crate::support::Harness;

#[tokio::test]
async fn invalid_arguments_never_dispatch_an_mcp_call() {
    let harness = Harness::start("success", McpLimits::default(), 2_000).await;

    assert!(
        harness
            .registry
            .execute(
                Harness::invocation(serde_json::json!({"wrong":true})),
                CancellationScope::new(),
            )
            .is_err()
    );
    assert!(harness.marker_text().is_empty());
    harness.shutdown().await;
}

#[tokio::test]
async fn cancellation_sends_protocol_cancel_and_terminates_once() {
    let harness = Harness::start("cancel", McpLimits::default(), 2_000).await;
    let cancellation = CancellationScope::new();
    let mut stream = harness.execute(cancellation.clone());
    let poll = tokio::spawn(async move { stream.next().await.unwrap() });

    while harness.marker_text().is_empty() {
        tokio::task::yield_now().await;
    }
    cancellation.cancel();
    let terminal = tokio::time::timeout(Duration::from_secs(1), poll)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(
        terminal,
        ToolExecutionEvent::Failed(ref failure)
            if failure.code() == ToolExecutionFailureCode::Cancelled
    ));
    wait_for_cancel_marker(&harness).await;
    assert_eq!(harness.marker_text(), "called\ncancelled\n");
    harness.shutdown().await;
}

#[tokio::test]
async fn dropping_a_dispatched_stream_still_sends_protocol_cancel() {
    let harness = Harness::start("cancel", McpLimits::default(), 2_000).await;
    let stream = harness.execute(CancellationScope::new());
    let poll = tokio::spawn(async move {
        let mut stream = stream;
        stream.next().await
    });

    while harness.marker_text().is_empty() {
        tokio::task::yield_now().await;
    }
    poll.abort();
    let _ = poll.await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !harness.marker_text().contains("cancelled") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    assert_eq!(harness.marker_text(), "called\ncancelled\n");
    harness.shutdown().await;
}

#[tokio::test]
async fn timeout_cancels_dispatched_request_without_retry() {
    let harness = Harness::start("timeout", McpLimits::default(), 75).await;
    let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
        .await
        .unwrap();

    assert_eq!(collected.report().terminal_kind(), ToolTerminalKind::Failed);
    wait_for_cancel_marker(&harness).await;
    assert_eq!(harness.marker_text(), "called\ncancelled\n");
    harness.shutdown().await;
}

#[tokio::test]
async fn progress_flood_is_bounded_and_terminal() {
    let limits = McpLimits::default().with_max_progress_events(2).unwrap();
    let harness = Harness::start("flood", limits, 2_000).await;
    let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
        .await
        .unwrap();

    assert!(collected.report().progress_count() <= 2);
    assert_eq!(collected.report().terminal_kind(), ToolTerminalKind::Failed);
    assert_eq!(
        collected.report().failure_code(),
        Some(ToolExecutionFailureCode::InvalidOutput)
    );
    harness.shutdown().await;
}

#[tokio::test]
async fn invalid_structured_output_and_mcp_error_are_typed_failures() {
    for (scenario, expected) in [
        ("schema", ToolExecutionFailureCode::InvalidOutput),
        ("is-error", ToolExecutionFailureCode::ExecutionFailed),
    ] {
        let harness = Harness::start(scenario, McpLimits::default(), 2_000).await;
        let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
            .await
            .unwrap();

        assert_eq!(
            collected.report().failure_code(),
            Some(expected),
            "{scenario}"
        );
        harness.shutdown().await;
    }
}

#[tokio::test]
async fn mismatched_response_id_fails_without_a_terminal_success() {
    let harness = Harness::start("mismatch", McpLimits::default(), 2_000).await;
    let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
        .await
        .unwrap();

    assert_eq!(collected.report().terminal_kind(), ToolTerminalKind::Failed);
    let details = match collected.events().last().unwrap() {
        ToolExecutionEvent::Failed(failure) => failure.details(),
        _ => panic!("expected failure"),
    };
    assert_eq!(
        details
            .get("dev.tea-rs.mcp")
            .and_then(|value| value.get("code")),
        Some(&serde_json::json!(McpErrorCode::Transport))
    );
    let _ = harness.client.shutdown().await;
    let _ = std::fs::remove_file(harness.marker);
}

#[tokio::test]
async fn duplicate_post_terminal_response_invalidates_the_transport() {
    let harness = Harness::start("duplicate", McpLimits::default(), 2_000).await;
    let collected = collect_tool_execution(harness.execute(CancellationScope::new()))
        .await
        .unwrap();

    assert_eq!(
        collected.report().terminal_kind(),
        ToolTerminalKind::Finished
    );
    tokio::task::yield_now().await;
    let error = harness
        .client
        .probe(Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(error.code(), McpErrorCode::Transport);
    let _ = harness.client.shutdown().await;
    let _ = std::fs::remove_file(harness.marker);
}

async fn wait_for_cancel_marker(harness: &Harness) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !harness.marker_text().contains("cancelled") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
