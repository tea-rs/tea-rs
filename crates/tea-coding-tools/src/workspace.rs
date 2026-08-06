use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::path::ValidatedRelativePath;
use crate::{WorkspacePathError, WorkspacePathErrorCode};

/// Validated authority to resolve tool paths beneath one canonical directory.
///
/// Construction resolves symlinks in the supplied host path and requires an
/// existing directory. Model-supplied paths are never interpreted relative to
/// the process current directory.
///
/// # TOCTOU limits
///
/// Path-based host APIs cannot make resolution and a later open or rename one
/// atomic operation. Callers performing mutations must invoke
/// [`Self::revalidate_mutation`] immediately before commit. Executors should
/// additionally prefer descriptor-relative operations where a safe portable
/// implementation is available. Unix identity checks use device/inode. The
/// portable stable Windows metadata APIs used here provide a best-effort
/// fingerprint instead of a file index. This capability fails closed on
/// detected replacement, but it does not claim to eliminate every filesystem
/// race.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    host_path: PathBuf,
    identity: ObjectIdentity,
}

impl WorkspaceRoot {
    /// Constructs a capability from an existing host directory.
    ///
    /// # Errors
    ///
    /// Returns a stable bounded error if the path is absent, is not a
    /// directory, or cannot be resolved safely.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, WorkspacePathError> {
        let host_path = fs::canonicalize(path.as_ref()).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                WorkspacePathError::new(WorkspacePathErrorCode::WorkspaceNotFound)
            } else {
                WorkspacePathError::new(WorkspacePathErrorCode::FilesystemFailure)
            }
        })?;
        let metadata = fs::metadata(&host_path)
            .map_err(|_| WorkspacePathError::new(WorkspacePathErrorCode::FilesystemFailure))?;
        if !metadata.is_dir() {
            return Err(WorkspacePathError::new(
                WorkspacePathErrorCode::WorkspaceNotDirectory,
            ));
        }
        Ok(Self {
            host_path,
            identity: ObjectIdentity::from_metadata(&metadata),
        })
    }

    /// Returns the canonical host directory represented by this capability.
    #[must_use]
    pub fn host_path(&self) -> &Path {
        &self.host_path
    }

    /// Resolves a model path that must already exist.
    ///
    /// The returned display path is normalized and workspace-relative. The
    /// host path is canonical and has passed a component-wise containment
    /// check against this capability.
    ///
    /// # Errors
    ///
    /// Rejects malformed, absolute, traversing, missing, unresolvable, or
    /// workspace-escaping paths.
    pub fn resolve_existing(&self, path: &str) -> Result<ResolvedExistingPath, WorkspacePathError> {
        self.verify_root()?;
        let relative = ValidatedRelativePath::parse(path)?;
        let candidate = relative.join_to(&self.host_path);
        let host_path = fs::canonicalize(&candidate).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                WorkspacePathError::new(WorkspacePathErrorCode::TargetNotFound)
            } else {
                WorkspacePathError::new(WorkspacePathErrorCode::UnresolvableTarget)
            }
        })?;
        self.require_contained(&host_path)?;
        let metadata = fs::metadata(&host_path)
            .map_err(|_| WorkspacePathError::new(WorkspacePathErrorCode::UnresolvableTarget))?;
        Ok(ResolvedExistingPath {
            display_path: relative.display().to_owned(),
            host_path,
            workspace_root: self.host_path.clone(),
            identity: ObjectIdentity::from_metadata(&metadata),
        })
    }

    /// Revalidates an existing target immediately before opening it.
    ///
    /// # Errors
    ///
    /// Returns `CapabilityMismatch` for a value from another workspace and
    /// `PathChanged` if resolution or filesystem identity changed.
    pub fn revalidate_existing(
        &self,
        target: &ResolvedExistingPath,
    ) -> Result<(), WorkspacePathError> {
        if target.workspace_root != self.host_path {
            return Err(WorkspacePathError::new(
                WorkspacePathErrorCode::CapabilityMismatch,
            ));
        }
        let current = self
            .resolve_existing(target.display_path())
            .map_err(|error| match error.code() {
                WorkspacePathErrorCode::TargetNotFound
                | WorkspacePathErrorCode::UnresolvableTarget => {
                    WorkspacePathError::new(WorkspacePathErrorCode::PathChanged)
                }
                _ => error,
            })?;
        if current.host_path != target.host_path || current.identity != target.identity {
            return Err(WorkspacePathError::new(WorkspacePathErrorCode::PathChanged));
        }
        Ok(())
    }

    /// Verifies that an opened file handle identifies the resolved target.
    ///
    /// Call this with metadata obtained from the opened file handle, not by
    /// looking up its path again. It closes the validation-to-open race for the
    /// object identity on platforms that expose stable file identifiers.
    ///
    /// # Errors
    ///
    /// Returns `PathChanged` if the opened object differs from the resolved
    /// target, or `CapabilityMismatch` for a token from another workspace.
    pub fn verify_opened_existing(
        &self,
        target: &ResolvedExistingPath,
        metadata: &fs::Metadata,
    ) -> Result<(), WorkspacePathError> {
        if target.workspace_root != self.host_path {
            return Err(WorkspacePathError::new(
                WorkspacePathErrorCode::CapabilityMismatch,
            ));
        }
        self.verify_root()?;
        if ObjectIdentity::from_metadata(metadata) != target.identity {
            return Err(WorkspacePathError::new(WorkspacePathErrorCode::PathChanged));
        }
        Ok(())
    }

    /// Resolves an existing or prospective mutation target.
    ///
    /// The workspace root itself is not a valid mutation target. Existing
    /// targets are canonicalized. For a prospective target, the nearest
    /// existing ancestor is canonicalized, checked as a directory, and checked
    /// for workspace containment before unresolved components are appended.
    ///
    /// # Errors
    ///
    /// Rejects malformed, absolute, traversing, unresolvable, root-targeting,
    /// non-directory-parent, or workspace-escaping paths.
    pub fn resolve_mutation(&self, path: &str) -> Result<ResolvedMutationPath, WorkspacePathError> {
        self.verify_root()?;
        let relative = ValidatedRelativePath::parse(path)?;
        if relative.display() == "." {
            return Err(WorkspacePathError::new(
                WorkspacePathErrorCode::WorkspaceRootMutation,
            ));
        }
        let candidate = relative.join_to(&self.host_path);

        if let Ok(host_path) = fs::canonicalize(&candidate) {
            self.require_contained(&host_path)?;
            let metadata = fs::metadata(&host_path)
                .map_err(|_| WorkspacePathError::new(WorkspacePathErrorCode::UnresolvableTarget))?;
            Ok(ResolvedMutationPath {
                display_path: relative.display().to_owned(),
                host_path: host_path.clone(),
                workspace_root: self.host_path.clone(),
                target_existed: true,
                anchor_path: host_path,
                anchor_identity: ObjectIdentity::from_metadata(&metadata),
            })
        } else {
            if fs::symlink_metadata(&candidate).is_ok() {
                return Err(WorkspacePathError::new(
                    WorkspacePathErrorCode::UnresolvableTarget,
                ));
            }
            self.resolve_prospective(&relative, &candidate)
        }
    }

    /// Revalidates a mutation target immediately before its commit operation.
    ///
    /// This detects containment changes, target appearance/disappearance, and
    /// nearest-existing-ancestor replacement on supported platforms. A caller
    /// must discard the old value and resolve again after any failure.
    ///
    /// # Errors
    ///
    /// Returns `CapabilityMismatch` for a value from another workspace and a
    /// stable path error when the target no longer has the same resolution.
    pub fn revalidate_mutation(
        &self,
        target: &ResolvedMutationPath,
    ) -> Result<(), WorkspacePathError> {
        if target.workspace_root != self.host_path {
            return Err(WorkspacePathError::new(
                WorkspacePathErrorCode::CapabilityMismatch,
            ));
        }
        let current = self.resolve_mutation(target.display_path())?;
        if current.host_path != target.host_path
            || current.target_existed != target.target_existed
            || current.anchor_path != target.anchor_path
            || current.anchor_identity != target.anchor_identity
        {
            return Err(WorkspacePathError::new(WorkspacePathErrorCode::PathChanged));
        }
        Ok(())
    }

    fn resolve_prospective(
        &self,
        relative: &ValidatedRelativePath,
        candidate: &Path,
    ) -> Result<ResolvedMutationPath, WorkspacePathError> {
        let mut ancestor = candidate
            .parent()
            .ok_or_else(|| WorkspacePathError::new(WorkspacePathErrorCode::InvalidPath))?;
        loop {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) => {
                    let anchor_path = fs::canonicalize(ancestor).map_err(|_| {
                        WorkspacePathError::new(WorkspacePathErrorCode::UnresolvableTarget)
                    })?;
                    self.require_contained(&anchor_path)?;
                    let followed_metadata = fs::metadata(&anchor_path).map_err(|_| {
                        WorkspacePathError::new(WorkspacePathErrorCode::UnresolvableTarget)
                    })?;
                    if !followed_metadata.is_dir() {
                        return Err(WorkspacePathError::new(
                            WorkspacePathErrorCode::ParentNotDirectory,
                        ));
                    }
                    let suffix = candidate.strip_prefix(ancestor).map_err(|_| {
                        WorkspacePathError::new(WorkspacePathErrorCode::UnresolvableTarget)
                    })?;
                    return Ok(ResolvedMutationPath {
                        display_path: relative.display().to_owned(),
                        host_path: anchor_path.join(suffix),
                        workspace_root: self.host_path.clone(),
                        target_existed: false,
                        anchor_path,
                        anchor_identity: ObjectIdentity::from_metadata(&metadata),
                    });
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    ancestor = ancestor.parent().ok_or_else(|| {
                        WorkspacePathError::new(WorkspacePathErrorCode::UnresolvableTarget)
                    })?;
                }
                Err(_) => {
                    return Err(WorkspacePathError::new(
                        WorkspacePathErrorCode::FilesystemFailure,
                    ));
                }
            }
        }
    }

    fn verify_root(&self) -> Result<(), WorkspacePathError> {
        let current = fs::canonicalize(&self.host_path)
            .map_err(|_| WorkspacePathError::new(WorkspacePathErrorCode::FilesystemFailure))?;
        let metadata = fs::metadata(&current)
            .map_err(|_| WorkspacePathError::new(WorkspacePathErrorCode::FilesystemFailure))?;
        if current != self.host_path
            || !metadata.is_dir()
            || ObjectIdentity::from_metadata(&metadata) != self.identity
        {
            return Err(WorkspacePathError::new(WorkspacePathErrorCode::PathChanged));
        }
        Ok(())
    }

    fn require_contained(&self, path: &Path) -> Result<(), WorkspacePathError> {
        if path.starts_with(&self.host_path) {
            Ok(())
        } else {
            Err(WorkspacePathError::new(
                WorkspacePathErrorCode::OutsideWorkspace,
            ))
        }
    }
}

/// Verified existing workspace target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExistingPath {
    display_path: String,
    host_path: PathBuf,
    workspace_root: PathBuf,
    identity: ObjectIdentity,
}

impl ResolvedExistingPath {
    /// Returns the normalized workspace-relative path safe for model/UI output.
    #[must_use]
    pub fn display_path(&self) -> &str {
        &self.display_path
    }

    /// Returns the canonical contained host path.
    #[must_use]
    pub fn host_path(&self) -> &Path {
        &self.host_path
    }
}

/// Verified existing or prospective workspace mutation target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMutationPath {
    display_path: String,
    host_path: PathBuf,
    workspace_root: PathBuf,
    target_existed: bool,
    anchor_path: PathBuf,
    anchor_identity: ObjectIdentity,
}

impl ResolvedMutationPath {
    /// Returns the normalized workspace-relative path safe for model/UI output.
    #[must_use]
    pub fn display_path(&self) -> &str {
        &self.display_path
    }

    /// Returns the verified contained host path.
    ///
    /// For prospective targets this path is formed by appending unresolved
    /// components to the canonical nearest existing parent.
    #[must_use]
    pub fn host_path(&self) -> &Path {
        &self.host_path
    }

    /// Reports whether the final target existed when this value was resolved.
    #[must_use]
    pub const fn target_existed_at_resolution(&self) -> bool {
        self.target_existed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    file_attributes: u32,
    #[cfg(windows)]
    creation_time: u64,
    #[cfg(windows)]
    last_write_time: u64,
    #[cfg(windows)]
    file_size: u64,
    #[cfg(not(any(unix, windows)))]
    length: u64,
    #[cfg(not(any(unix, windows)))]
    modified: Option<std::time::SystemTime>,
}

impl ObjectIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            Self {
                file_attributes: metadata.file_attributes(),
                creation_time: metadata.creation_time(),
                last_write_time: metadata.last_write_time(),
                file_size: metadata.file_size(),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
            }
        }
    }
}
