use crate::common;

use std::fs;

use serde_json::json;
use tea_coding_tools::MAX_WRITE_BYTES;
use tea_protocol::{CodeChangeLineKind, ToolPresentation};

use common::{TestDirectory, edit_registry, execute, failed, finished};

#[tokio::test(flavor = "current_thread")]
async fn edit_replaces_one_exact_match_atomically() {
    let temp = TestDirectory::new();
    fs::write(temp.path().join("file.txt"), "before old after\n").unwrap();
    let registry = edit_registry(temp.workspace());

    let events = execute(
        &registry,
        "edit",
        json!({"path":"file.txt","oldText":"old","newText":"new"}),
    )
    .await;
    let result = finished(&events);
    assert_eq!(result.output()["path"], "file.txt");
    assert_eq!(result.output()["replacements"], 1);
    assert_eq!(result.output()["writtenBytes"], 17);
    let Some(ToolPresentation::CodeChange(change)) = result.presentation() else {
        panic!("successful edit must include a structured code-change presentation");
    };
    assert_eq!(change.path(), "file.txt");
    assert_eq!(change.first_changed_line(), Some(1));
    assert!(
        change
            .patch()
            .is_some_and(|patch| patch.contains("-before old after"))
    );
    assert!(
        change
            .hunks()
            .iter()
            .flat_map(tea_protocol::CodeChangeHunk::lines)
            .any(|line| {
                line.kind() == CodeChangeLineKind::Addition && line.text() == "before new after"
            })
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("file.txt")).unwrap(),
        "before new after\n"
    );
}

#[test]
fn edit_preview_uses_current_content_without_mutating_the_file() {
    let temp = TestDirectory::new();
    let target = temp.path().join("file.txt");
    fs::write(&target, "before old after\n").unwrap();
    let registry = edit_registry(temp.workspace());
    let invocation = registry
        .validate(common::invocation(
            "edit",
            json!({"path":"file.txt","oldText":"old","newText":"new"}),
        ))
        .unwrap();

    let Some(ToolPresentation::CodeChange(change)) = registry.preview_validated(&invocation) else {
        panic!("edit preview must produce a structured code change");
    };
    assert_eq!(change.path(), "file.txt");
    assert_eq!(change.first_changed_line(), Some(1));
    assert!(
        change
            .patch()
            .is_some_and(|patch| patch.contains("+before new after"))
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "before old after\n");
}

#[tokio::test(flavor = "current_thread")]
async fn edit_rejects_zero_and_ambiguous_matches_without_mutation() {
    let temp = TestDirectory::new();
    let target = temp.path().join("file.txt");
    fs::write(&target, "same same\n").unwrap();
    let registry = edit_registry(temp.workspace());

    for arguments in [
        json!({"path":"file.txt","oldText":"missing","newText":"new"}),
        json!({"path":"file.txt","oldText":"same","newText":"new"}),
    ] {
        let events = execute(&registry, "edit", arguments).await;
        failed(&events);
        assert_eq!(fs::read_to_string(&target).unwrap(), "same same\n");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn edit_allows_an_explicit_expected_replacement_count() {
    let temp = TestDirectory::new();
    let target = temp.path().join("file.txt");
    fs::write(&target, "same same\n").unwrap();
    let registry = edit_registry(temp.workspace());

    let events = execute(
        &registry,
        "edit",
        json!({
            "path":"file.txt",
            "oldText":"same",
            "newText":"new",
            "expectedReplacements":2
        }),
    )
    .await;
    assert_eq!(finished(&events).output()["replacements"], 2);
    assert_eq!(fs::read_to_string(target).unwrap(), "new new\n");
}

#[tokio::test(flavor = "current_thread")]
async fn edit_expected_count_mismatch_and_oversize_are_noops() {
    let temp = TestDirectory::new();
    let target = temp.path().join("file.txt");
    fs::write(&target, "x x\n").unwrap();
    let registry = edit_registry(temp.workspace());

    let mismatch = execute(
        &registry,
        "edit",
        json!({
            "path":"file.txt","oldText":"x","newText":"y","expectedReplacements":3
        }),
    )
    .await;
    failed(&mismatch);
    assert_eq!(fs::read_to_string(&target).unwrap(), "x x\n");

    let oversize = execute(
        &registry,
        "edit",
        json!({"path":"file.txt","oldText":"x","newText":"界".repeat(MAX_WRITE_BYTES / 3 + 1)}),
    )
    .await;
    failed(&oversize);
    assert_eq!(fs::read_to_string(&target).unwrap(), "x x\n");
}

#[tokio::test(flavor = "current_thread")]
async fn pre_cancelled_edit_does_not_modify_the_file() {
    use futures_util::StreamExt;
    use tea_control::CancellationScope;
    use tea_tools::{ToolExecutionEvent, ToolExecutionFailureCode};

    let temp = TestDirectory::new();
    let target = temp.path().join("file.txt");
    fs::write(&target, "old").unwrap();
    let registry = edit_registry(temp.workspace());
    let cancellation = CancellationScope::new();
    cancellation.cancel();
    let events = registry
        .execute(
            common::invocation(
                "edit",
                json!({"path":"file.txt","oldText":"old","newText":"new"}),
            ),
            cancellation,
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(events.as_slice(), [ToolExecutionEvent::Failed(failure)] if failure.code() == ToolExecutionFailureCode::Cancelled)
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "old");
}

#[tokio::test(flavor = "current_thread")]
async fn edit_rejects_binary_directory_and_missing_files() {
    let temp = TestDirectory::new();
    fs::write(temp.path().join("binary"), [0, 1, 0]).unwrap();
    fs::create_dir(temp.path().join("directory")).unwrap();
    let registry = edit_registry(temp.workspace());

    for path in ["binary", "directory", "missing"] {
        let events = execute(
            &registry,
            "edit",
            json!({"path":path,"oldText":"x","newText":"y"}),
        )
        .await;
        failed(&events);
    }
}
