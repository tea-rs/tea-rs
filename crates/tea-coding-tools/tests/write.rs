use crate::common;

use std::fs;

use serde_json::json;
use tea_coding_tools::MAX_WRITE_BYTES;
use tea_protocol::{CodeChangeKind, CodeChangeLineKind, ToolPresentation};

use common::{TestDirectory, execute, failed, finished, write_registry};

#[test]
fn write_preview_uses_current_content_without_mutating_the_workspace() {
    let temp = TestDirectory::new();
    let existing = temp.path().join("existing.txt");
    fs::write(&existing, "before\n").unwrap();
    let registry = write_registry(temp.workspace());

    let update = registry
        .validate(common::invocation(
            "write",
            json!({"path":"existing.txt","content":"after\n"}),
        ))
        .unwrap();
    let Some(ToolPresentation::CodeChange(change)) = registry.preview_validated(&update) else {
        panic!("text overwrite must produce a structured code-change preview");
    };
    assert_eq!(change.kind(), CodeChangeKind::Update);
    assert!(
        change
            .patch()
            .is_some_and(|patch| patch.contains("-before") && patch.contains("+after"))
    );
    assert_eq!(fs::read_to_string(&existing).unwrap(), "before\n");

    let create = registry
        .validate(common::invocation(
            "write",
            json!({"path":"created.txt","content":"created\n"}),
        ))
        .unwrap();
    let Some(ToolPresentation::CodeChange(change)) = registry.preview_validated(&create) else {
        panic!("new text file must produce a structured code-change preview");
    };
    assert_eq!(change.kind(), CodeChangeKind::Create);
    assert!(
        change
            .hunks()
            .iter()
            .flat_map(tea_protocol::CodeChangeHunk::lines)
            .any(|line| line.kind() == CodeChangeLineKind::Addition && line.text() == "created")
    );
    assert!(!temp.path().join("created.txt").exists());
}

#[tokio::test(flavor = "current_thread")]
async fn write_atomically_creates_and_replaces_utf8_files() {
    let temp = TestDirectory::new();
    fs::create_dir(temp.path().join("src")).unwrap();
    let registry = write_registry(temp.workspace());

    let created = execute(
        &registry,
        "write",
        json!({"path":"src/new.txt","content":"hello 世界\n"}),
    )
    .await;
    let result = finished(&created);
    assert_eq!(result.output()["path"], "src/new.txt");
    assert_eq!(result.output()["writtenBytes"], 13);
    assert_eq!(result.output()["created"], true);
    let Some(ToolPresentation::CodeChange(change)) = result.presentation() else {
        panic!("created text file must include a structured code-change presentation");
    };
    assert_eq!(change.kind(), CodeChangeKind::Create);
    assert!(
        change
            .hunks()
            .iter()
            .flat_map(tea_protocol::CodeChangeHunk::lines)
            .any(|line| line.kind() == CodeChangeLineKind::Addition && line.text() == "hello 世界")
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/new.txt")).unwrap(),
        "hello 世界\n"
    );

    let replaced = execute(
        &registry,
        "write",
        json!({"path":"src/new.txt","content":"replacement"}),
    )
    .await;
    let result = finished(&replaced);
    assert_eq!(result.output()["created"], false);
    let Some(ToolPresentation::CodeChange(change)) = result.presentation() else {
        panic!("updated text file must include a structured code-change presentation");
    };
    assert_eq!(change.kind(), CodeChangeKind::Update);
    assert!(
        change
            .hunks()
            .iter()
            .flat_map(tea_protocol::CodeChangeHunk::lines)
            .any(|line| line.kind() == CodeChangeLineKind::Deletion && line.text() == "hello 世界")
    );
    assert!(
        change
            .hunks()
            .iter()
            .flat_map(tea_protocol::CodeChangeHunk::lines)
            .any(|line| line.kind() == CodeChangeLineKind::Addition && line.text() == "replacement")
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/new.txt")).unwrap(),
        "replacement"
    );
    assert!(fs::read_dir(temp.path().join("src")).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tea-tmp-")
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn overwriting_binary_content_succeeds_without_a_text_diff() {
    let temp = TestDirectory::new();
    fs::write(temp.path().join("binary.bin"), [0_u8, 0xFF]).unwrap();
    let registry = write_registry(temp.workspace());

    let events = execute(
        &registry,
        "write",
        json!({"path":"binary.bin","content":"replacement"}),
    )
    .await;
    let result = finished(&events);
    assert!(result.presentation().is_none());
    assert_eq!(
        fs::read_to_string(temp.path().join("binary.bin")).unwrap(),
        "replacement"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn replacing_an_existing_file_preserves_its_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TestDirectory::new();
    let target = temp.path().join("script.sh");
    fs::write(&target, b"old").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
    let registry = write_registry(temp.workspace());

    finished(
        &execute(
            &registry,
            "write",
            json!({"path":"script.sh","content":"new"}),
        )
        .await,
    );
    assert_eq!(
        fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o750
    );
}

#[tokio::test(flavor = "current_thread")]
async fn write_rejects_missing_parent_oversize_directory_and_workspace_root() {
    let temp = TestDirectory::new();
    fs::create_dir(temp.path().join("directory")).unwrap();
    let registry = write_registry(temp.workspace());

    for arguments in [
        json!({"path":"missing/child","content":"x"}),
        json!({"path":"directory","content":"x"}),
        json!({"path":".","content":"x"}),
        json!({"path":"large","content":"界".repeat(MAX_WRITE_BYTES / 3 + 1)}),
    ] {
        let events = execute(&registry, "write", arguments).await;
        failed(&events);
    }
    assert!(!temp.path().join("missing").exists());
    assert!(temp.path().join("directory").is_dir());
}

#[tokio::test(flavor = "current_thread")]
async fn pre_cancelled_write_does_not_create_a_file() {
    use futures_util::StreamExt;
    use tea_control::CancellationScope;
    use tea_tools::{ToolExecutionEvent, ToolExecutionFailureCode};

    let temp = TestDirectory::new();
    let registry = write_registry(temp.workspace());
    let cancellation = CancellationScope::new();
    cancellation.cancel();
    let events = registry
        .execute(
            common::invocation("write", json!({"path":"new.txt","content":"data"})),
            cancellation,
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(events.as_slice(), [ToolExecutionEvent::Failed(failure)] if failure.code() == ToolExecutionFailureCode::Cancelled)
    );
    assert!(!temp.path().join("new.txt").exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn write_rejects_symlink_escape_without_modifying_outside_content() {
    use std::os::unix::fs::symlink;

    let temp = TestDirectory::new();
    let outside = temp
        .path()
        .parent()
        .unwrap()
        .join(format!("tea-coding-outside-write-{}", std::process::id()));
    let _ = fs::remove_file(&outside);
    fs::write(&outside, b"outside").unwrap();
    symlink(&outside, temp.path().join("link")).unwrap();
    let registry = write_registry(temp.workspace());

    let events = execute(
        &registry,
        "write",
        json!({"path":"link","content":"overwrite"}),
    )
    .await;
    failed(&events);
    assert_eq!(fs::read(&outside).unwrap(), b"outside");
    fs::remove_file(outside).unwrap();
}
