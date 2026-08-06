use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use tea_coding::{
    CodingErrorCode, InteractionMode, PersistedTrustDecision, ProjectAccess, ProjectTrustStore,
    TrustRequest,
};

static ID: AtomicU64 = AtomicU64::new(0);

fn roots() -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "tea-coding-trust-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    (root, workspace)
}

#[test]
fn noninteractive_default_fails_closed_and_once_is_ephemeral() {
    let (root, workspace) = roots();
    let store = ProjectTrustStore::new(root.join("state/trust.json"));
    assert_eq!(
        store
            .resolve(
                &workspace,
                TrustRequest::Default,
                InteractionMode::NonInteractive,
            )
            .unwrap_err()
            .code(),
        CodingErrorCode::ProjectNotTrusted
    );
    assert_eq!(
        store
            .resolve(
                &workspace,
                TrustRequest::TrustOnce,
                InteractionMode::NonInteractive,
            )
            .unwrap(),
        ProjectAccess::Trusted
    );
    assert_eq!(store.get(&workspace).unwrap(), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn persisted_trust_survives_reopen_and_ignore_never_loads_project() {
    let (root, workspace) = roots();
    let path = root.join("state/trust.json");
    let store = ProjectTrustStore::new(&path);
    assert_eq!(
        store
            .resolve(
                &workspace,
                TrustRequest::TrustPersisted,
                InteractionMode::NonInteractive,
            )
            .unwrap(),
        ProjectAccess::Trusted
    );
    let reopened = ProjectTrustStore::new(&path);
    assert_eq!(
        reopened
            .resolve(
                &workspace,
                TrustRequest::Default,
                InteractionMode::NonInteractive,
            )
            .unwrap(),
        ProjectAccess::Trusted
    );
    reopened
        .set(&workspace, PersistedTrustDecision::Ignored)
        .unwrap();
    assert_eq!(
        reopened
            .resolve(
                &workspace,
                TrustRequest::Default,
                InteractionMode::Interactive,
            )
            .unwrap(),
        ProjectAccess::Ignored
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn interactive_undecided_returns_ask_without_reading_project_files() {
    let (root, workspace) = roots();
    let store = ProjectTrustStore::new(root.join("state/trust.json"));
    assert_eq!(
        store
            .resolve(
                &workspace,
                TrustRequest::Default,
                InteractionMode::Interactive,
            )
            .unwrap(),
        ProjectAccess::Ask
    );
    fs::remove_dir_all(root).unwrap();
}
