use futures_util::stream;
use serde_json::json;
use tea_protocol::ContentBlock;
use tea_testkit::{ToolConformanceError, ToolTerminalKind, collect_tool_execution};
use tea_tools::{
    ToolExecutionEvent, ToolExecutionFailure, ToolExecutionFailureCode, ToolProgress, ToolResult,
};

#[tokio::test(flavor = "current_thread")]
async fn collector_reports_progress_and_success() {
    let stream = Box::pin(stream::iter([
        ToolExecutionEvent::Progress(ToolProgress::new("half", 1, Some(2)).unwrap()),
        ToolExecutionEvent::Finished(
            ToolResult::new(
                vec![ContentBlock::text("done").unwrap()],
                json!({"value":"done"}),
            )
            .unwrap(),
        ),
    ]));
    let collected = collect_tool_execution(stream).await.unwrap();
    assert_eq!(collected.events().len(), 2);
    assert_eq!(collected.report().event_count(), 2);
    assert_eq!(collected.report().progress_count(), 1);
    assert_eq!(
        collected.report().terminal_kind(),
        ToolTerminalKind::Finished
    );
    assert_eq!(collected.report().failure_code(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn collector_reports_typed_failure() {
    let stream = Box::pin(stream::iter([ToolExecutionEvent::Failed(
        ToolExecutionFailure::cancelled(),
    )]));
    let report = collect_tool_execution(stream).await.unwrap().report();
    assert_eq!(report.terminal_kind(), ToolTerminalKind::Failed);
    assert_eq!(
        report.failure_code(),
        Some(ToolExecutionFailureCode::Cancelled)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn collector_rejects_missing_and_duplicate_terminal_events() {
    let missing = Box::pin(stream::iter([ToolExecutionEvent::Progress(
        ToolProgress::new("pending", 0, None).unwrap(),
    )]));
    assert!(matches!(
        collect_tool_execution(missing).await.unwrap_err(),
        ToolConformanceError::Stream(_)
    ));

    let duplicate = Box::pin(stream::iter([
        ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled()),
        ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled()),
    ]));
    assert!(matches!(
        collect_tool_execution(duplicate).await.unwrap_err(),
        ToolConformanceError::Stream(_)
    ));
}
