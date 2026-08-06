use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::file::atomic_write_if_unchanged;
use crate::{FileToolErrorCode, WorkspaceRoot};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn stale_edit_commit_is_rejected_without_modifying_new_content() {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("tea-coding-stale-edit-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"original").unwrap();
    let workspace = WorkspaceRoot::new(&root).unwrap();
    let existing = workspace.resolve_existing("file").unwrap();
    let mutation = workspace.resolve_mutation("file").unwrap();

    fs::write(root.join("file"), b"concurrent").unwrap();
    let error = atomic_write_if_unchanged(&workspace, &existing, &mutation, "original", b"edited")
        .unwrap_err();
    assert!(matches!(
        error.code(),
        FileToolErrorCode::InvalidPath | FileToolErrorCode::PathChanged
    ));
    assert_eq!(fs::read(root.join("file")).unwrap(), b"concurrent");
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tea-tmp-")
    }));
    fs::remove_dir_all(root).unwrap();
}
