use std::fs;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use tea_coding::ProjectAccess;
use tea_coding::resources::ResourceCatalog;
use tea_context::{ContextProvider, ContextRequest, WorkspaceInstructionProvider};
use tea_protocol::{ProfileId, ProtocolMetadata, SessionId};

static ID: AtomicU64 = AtomicU64::new(0);

#[tokio::test(flavor = "current_thread")]
async fn context_order_is_deterministic_and_untrusted_projects_are_not_read() {
    let root = std::env::temp_dir().join(format!(
        "coding-context-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("repo/sub");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(root.join("AGENTS.md"), "root agents").unwrap();
    fs::write(root.join("repo/CLAUDE.md"), "repo claude").unwrap();
    fs::write(workspace.join("AGENTS.md"), "sub agents").unwrap();

    let ignored = ResourceCatalog::discover(
        &root,
        &workspace,
        ProjectAccess::Ignored,
        &[],
        &[],
        None,
        None,
    )
    .unwrap();
    assert!(ignored.context().is_empty());
    let trusted = ResourceCatalog::discover(
        &root,
        &workspace,
        ProjectAccess::Trusted,
        &[],
        &[],
        None,
        None,
    )
    .unwrap();
    assert_eq!(trusted.context().len(), 3);
    let provider = WorkspaceInstructionProvider::new(trusted.context().to_vec()).unwrap();
    let modules = provider
        .provide(
            ContextRequest::new(
                ProfileId::from_str("coding-agent").unwrap(),
                SessionId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap(),
                None,
                Vec::new(),
                ProtocolMetadata::default(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(modules[0].segments()[0].content(), "root agents");
    assert_eq!(modules[0].segments()[1].content(), "repo claude");
    assert_eq!(modules[0].segments()[2].content(), "sub agents");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn blank_context_files_are_ignored() {
    let root = std::env::temp_dir().join(format!(
        "coding-context-blank-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("AGENTS.md"), "").unwrap();
    fs::write(root.join("CLAUDE.md"), " \n\t\r\n").unwrap();

    let catalog =
        ResourceCatalog::discover(&root, &root, ProjectAccess::Trusted, &[], &[], None, None)
            .unwrap();

    assert!(catalog.context().is_empty());
    assert!(catalog.diagnostics().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn trusted_context_symlink_cannot_escape_boundary() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!(
        "coding-context-symlink-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let outside = root.with_extension("outside");
    fs::create_dir_all(&root).unwrap();
    fs::write(&outside, "host secret").unwrap();
    symlink(&outside, root.join("AGENTS.md")).unwrap();
    assert!(
        ResourceCatalog::discover(&root, &root, ProjectAccess::Trusted, &[], &[], None, None,)
            .is_err()
    );
    fs::remove_dir_all(root).unwrap();
    fs::remove_file(outside).unwrap();
}
