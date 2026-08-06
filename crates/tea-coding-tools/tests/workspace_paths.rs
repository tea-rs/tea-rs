use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tea_coding_tools::{
    MAX_WORKSPACE_PATH_BYTES, MAX_WORKSPACE_PATH_COMPONENTS, WorkspacePathErrorCode, WorkspaceRoot,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tea-coding-workspace-paths-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn assert_code<T: std::fmt::Debug>(
    result: Result<T, tea_coding_tools::WorkspacePathError>,
    code: WorkspacePathErrorCode,
) {
    assert_eq!(result.unwrap_err().code(), code);
}

#[test]
fn workspace_root_requires_an_existing_directory_and_is_canonical() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let capability = WorkspaceRoot::new(root.join(".")).unwrap();
    assert_eq!(capability.host_path(), fs::canonicalize(&root).unwrap());

    assert_code(
        WorkspaceRoot::new(temp.path().join("missing")),
        WorkspacePathErrorCode::WorkspaceNotFound,
    );
    let file = temp.path().join("file");
    fs::write(&file, b"not a directory").unwrap();
    assert_code(
        WorkspaceRoot::new(file),
        WorkspacePathErrorCode::WorkspaceNotDirectory,
    );
}

#[test]
fn existing_targets_are_normalized_and_must_exist() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), b"pub fn example() {}\n").unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();

    let resolved = capability.resolve_existing("src/./lib.rs").unwrap();
    assert_eq!(resolved.display_path(), "src/lib.rs");
    assert_eq!(
        resolved.host_path(),
        fs::canonicalize(root.join("src/lib.rs")).unwrap()
    );

    let root_target = capability.resolve_existing(".").unwrap();
    assert_eq!(root_target.display_path(), ".");
    assert_eq!(root_target.host_path(), capability.host_path());

    assert_code(
        capability.resolve_existing("src/missing.rs"),
        WorkspacePathErrorCode::TargetNotFound,
    );
}

#[test]
fn model_paths_reject_absolute_parent_empty_and_platform_prefix_forms() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();

    let absolute = root.join("file").to_string_lossy().into_owned();
    assert_code(
        capability.resolve_existing(&absolute),
        WorkspacePathErrorCode::AbsolutePath,
    );
    for path in ["../outside", "safe/../../outside", "safe/../file"] {
        assert_code(
            capability.resolve_mutation(path),
            WorkspacePathErrorCode::ParentTraversal,
        );
    }
    for path in [
        "",
        "C:/Windows/system.ini",
        "C:\\Windows\\system.ini",
        "\\\\server\\share\\file",
    ] {
        assert_code(
            capability.resolve_mutation(path),
            WorkspacePathErrorCode::InvalidPath,
        );
    }
    assert_code(
        capability.resolve_mutation("line\nfeed"),
        WorkspacePathErrorCode::InvalidPath,
    );
}

#[test]
fn path_bytes_and_component_counts_are_bounded_before_filesystem_access() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();

    let too_long = "a".repeat(MAX_WORKSPACE_PATH_BYTES + 1);
    assert_code(
        capability.resolve_mutation(&too_long),
        WorkspacePathErrorCode::PathTooLong,
    );
    let too_many = std::iter::repeat_n("a", MAX_WORKSPACE_PATH_COMPONENTS + 1)
        .collect::<Vec<_>>()
        .join("/");
    assert_code(
        capability.resolve_mutation(&too_many),
        WorkspacePathErrorCode::TooManyComponents,
    );
}

#[test]
fn prospective_mutations_verify_the_nearest_existing_parent() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir_all(root.join("existing")).unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();

    let target = capability
        .resolve_mutation("existing/new/deep/file.txt")
        .unwrap();
    assert_eq!(target.display_path(), "existing/new/deep/file.txt");
    assert_eq!(
        target.host_path(),
        capability.host_path().join("existing/new/deep/file.txt")
    );
    assert!(!target.target_existed_at_resolution());
    capability.revalidate_mutation(&target).unwrap();

    fs::write(root.join("existing/file.txt"), b"old").unwrap();
    let existing = capability.resolve_mutation("existing/file.txt").unwrap();
    assert!(existing.target_existed_at_resolution());
    assert_eq!(
        existing.host_path(),
        fs::canonicalize(root.join("existing/file.txt")).unwrap()
    );
}

#[test]
fn mutation_rejects_the_workspace_root_and_a_file_as_missing_parent() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"content").unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();

    assert_code(
        capability.resolve_mutation("."),
        WorkspacePathErrorCode::WorkspaceRootMutation,
    );
    assert_code(
        capability.resolve_mutation("file/child"),
        WorkspacePathErrorCode::ParentNotDirectory,
    );
}

#[cfg(unix)]
#[test]
fn symlinks_must_resolve_inside_the_workspace() {
    use std::os::unix::fs::symlink;

    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("real/nested")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(root.join("real/nested/in.txt"), b"inside").unwrap();
    fs::write(outside.join("out.txt"), b"outside").unwrap();
    symlink(root.join("real"), root.join("inside-link")).unwrap();
    symlink(&outside, root.join("outside-link")).unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();

    let inside = capability
        .resolve_existing("inside-link/nested/in.txt")
        .unwrap();
    assert_eq!(inside.display_path(), "inside-link/nested/in.txt");
    assert_eq!(
        inside.host_path(),
        fs::canonicalize(root.join("real/nested/in.txt")).unwrap()
    );
    assert_code(
        capability.resolve_existing("outside-link/out.txt"),
        WorkspacePathErrorCode::OutsideWorkspace,
    );
    assert_code(
        capability.resolve_mutation("outside-link/new.txt"),
        WorkspacePathErrorCode::OutsideWorkspace,
    );
}

#[cfg(unix)]
#[test]
fn mutation_revalidation_detects_parent_replacement_escape() {
    use std::os::unix::fs::symlink;

    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("parent")).unwrap();
    fs::create_dir(&outside).unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();
    let target = capability.resolve_mutation("parent/new.txt").unwrap();

    fs::remove_dir(root.join("parent")).unwrap();
    symlink(&outside, root.join("parent")).unwrap();
    assert_code(
        capability.revalidate_mutation(&target),
        WorkspacePathErrorCode::OutsideWorkspace,
    );
}

#[test]
fn revalidation_detects_existing_target_replacement_and_prospective_target_appearance() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("existing.txt"), b"old").unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();
    let existing = capability.resolve_existing("existing.txt").unwrap();
    let existing_mutation = capability.resolve_mutation("existing.txt").unwrap();
    let prospective = capability.resolve_mutation("new.txt").unwrap();

    fs::remove_file(root.join("existing.txt")).unwrap();
    fs::write(root.join("existing.txt"), b"replacement").unwrap();
    assert_code(
        capability.revalidate_existing(&existing),
        WorkspacePathErrorCode::PathChanged,
    );
    assert_code(
        capability.revalidate_mutation(&existing_mutation),
        WorkspacePathErrorCode::PathChanged,
    );

    fs::write(root.join("new.txt"), b"appeared").unwrap();
    assert_code(
        capability.revalidate_mutation(&prospective),
        WorkspacePathErrorCode::PathChanged,
    );
}

#[test]
fn revalidation_detects_an_existing_target_disappearing() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("target.txt"), b"old").unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();
    let existing = capability.resolve_existing("target.txt").unwrap();
    let mutation = capability.resolve_mutation("target.txt").unwrap();

    fs::remove_file(root.join("target.txt")).unwrap();
    assert_code(
        capability.revalidate_existing(&existing),
        WorkspacePathErrorCode::PathChanged,
    );
    assert_code(
        capability.revalidate_mutation(&mutation),
        WorkspacePathErrorCode::PathChanged,
    );
}

#[cfg(unix)]
#[test]
fn dangling_symlinks_and_workspace_root_replacement_fail_closed() {
    use std::os::unix::fs::symlink;

    let temp = TestDirectory::new();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    symlink(root.join("absent"), root.join("dangling")).unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();
    assert_code(
        capability.resolve_existing("dangling"),
        WorkspacePathErrorCode::TargetNotFound,
    );
    assert_code(
        capability.resolve_mutation("dangling"),
        WorkspacePathErrorCode::UnresolvableTarget,
    );

    fs::remove_file(root.join("dangling")).unwrap();
    fs::remove_dir(&root).unwrap();
    fs::create_dir(&root).unwrap();
    assert_code(
        capability.resolve_mutation("new.txt"),
        WorkspacePathErrorCode::PathChanged,
    );
}

#[test]
fn mutation_revalidation_rejects_candidates_from_another_workspace() {
    let temp = TestDirectory::new();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    let first = WorkspaceRoot::new(first).unwrap();
    let second = WorkspaceRoot::new(second).unwrap();
    let target = first.resolve_mutation("new.txt").unwrap();

    assert_code(
        second.revalidate_mutation(&target),
        WorkspacePathErrorCode::CapabilityMismatch,
    );
}

#[test]
fn errors_are_machine_readable_bounded_and_do_not_echo_sensitive_paths() {
    let temp = TestDirectory::new();
    let root = temp.path().join("workspace-secret-name");
    fs::create_dir(&root).unwrap();
    let capability = WorkspaceRoot::new(&root).unwrap();
    let sensitive_input = "../outside-secret-name";
    let error = capability.resolve_existing(sensitive_input).unwrap_err();

    assert_eq!(error.code(), WorkspacePathErrorCode::ParentTraversal);
    assert!(error.message().len() <= 256);
    assert!(!error.message().contains("workspace-secret-name"));
    assert!(!error.message().contains("outside-secret-name"));
    assert!(
        !error
            .to_string()
            .contains(temp.path().to_string_lossy().as_ref())
    );
}
