use crate::common;

use std::fs;

use serde_json::json;
use tea_coding_tools::MAX_READ_BYTES;
use tea_tools::ToolExecutionFailureCode;

use common::{TestDirectory, execute, failed, finished, read_registry};

#[tokio::test(flavor = "current_thread")]
async fn read_returns_bounded_utf8_lines_with_offset_and_limit() {
    let temp = TestDirectory::new();
    fs::write(temp.path().join("notes.txt"), "zero\none\ntwo\nthree\n").unwrap();
    let registry = read_registry(temp.workspace());

    let events = execute(
        &registry,
        "read",
        json!({"path":"notes.txt","offset":2,"limit":2}),
    )
    .await;
    let result = finished(&events);
    assert_eq!(result.output()["path"], "notes.txt");
    assert_eq!(result.output()["content"], "one\ntwo\n");
    assert_eq!(result.output()["startLine"], 2);
    assert_eq!(result.output()["endLine"], 3);
    assert_eq!(result.output()["totalLines"], 4);
    assert_eq!(result.output()["truncated"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn read_supports_empty_text_and_eof_offsets_without_empty_content_blocks() {
    let temp = TestDirectory::new();
    fs::write(temp.path().join("empty.txt"), b"").unwrap();
    fs::write(temp.path().join("short.txt"), b"only\n").unwrap();
    let registry = read_registry(temp.workspace());

    let empty = execute(&registry, "read", json!({"path":"empty.txt"})).await;
    assert_eq!(finished(&empty).output()["content"], "");
    let eof = execute(&registry, "read", json!({"path":"short.txt","offset":3})).await;
    assert_eq!(finished(&eof).output()["content"], "");
}

#[tokio::test(flavor = "current_thread")]
async fn read_rejects_binary_invalid_utf8_directory_and_oversized_files() {
    let temp = TestDirectory::new();
    fs::write(temp.path().join("binary"), [0, 1, 2, 0]).unwrap();
    fs::write(temp.path().join("invalid"), [0xff, 0xfe]).unwrap();
    fs::create_dir(temp.path().join("directory")).unwrap();
    fs::write(temp.path().join("large"), vec![b'x'; MAX_READ_BYTES + 1]).unwrap();
    let registry = read_registry(temp.workspace());

    for path in ["binary", "invalid", "directory", "large"] {
        let events = execute(&registry, "read", json!({"path":path})).await;
        let failure = failed(&events);
        assert_eq!(failure.code(), ToolExecutionFailureCode::ExecutionFailed);
        assert!(
            !failure
                .message()
                .contains(temp.path().to_string_lossy().as_ref())
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn read_rejects_workspace_escape_and_honors_pre_cancelled_scope() {
    use futures_util::StreamExt;
    use tea_control::CancellationScope;
    use tea_tools::ToolExecutionEvent;

    let temp = TestDirectory::new();
    fs::write(temp.path().join("file"), b"content").unwrap();
    let registry = read_registry(temp.workspace());
    assert!(
        registry
            .execute(
                common::invocation("read", json!({"path":"../outside"})),
                CancellationScope::new()
            )
            .is_err()
    );

    let cancellation = CancellationScope::new();
    cancellation.cancel();
    let events = registry
        .execute(
            common::invocation("read", json!({"path":"file"})),
            cancellation,
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(events.as_slice(), [ToolExecutionEvent::Failed(failure)] if failure.code() == ToolExecutionFailureCode::Cancelled)
    );
}
