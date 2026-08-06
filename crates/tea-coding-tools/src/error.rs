use thiserror::Error;

/// Maximum UTF-8 bytes exposed by a workspace-path diagnostic.
pub const MAX_WORKSPACE_ERROR_MESSAGE_BYTES: usize = 256;

/// Stable machine-readable classification for workspace capability failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspacePathErrorCode {
    /// The requested workspace root does not exist.
    WorkspaceNotFound,
    /// The requested workspace root is not a directory.
    WorkspaceNotDirectory,
    /// An absolute model-supplied path was rejected.
    AbsolutePath,
    /// A model-supplied path contained a parent traversal component.
    ParentTraversal,
    /// A model-supplied path was empty, ambiguous, or platform-specific.
    InvalidPath,
    /// A model-supplied path exceeded its UTF-8 byte bound.
    PathTooLong,
    /// A model-supplied path exceeded its component-count bound.
    TooManyComponents,
    /// An existing target was required but was not found.
    TargetNotFound,
    /// A mutation attempted to target the workspace root itself.
    WorkspaceRootMutation,
    /// A resolved target or ancestor escaped the workspace.
    OutsideWorkspace,
    /// The nearest existing mutation ancestor was not a directory.
    ParentNotDirectory,
    /// A symlink or filesystem object could not be resolved safely.
    UnresolvableTarget,
    /// A mutation target changed after it was resolved.
    PathChanged,
    /// A resolved mutation target belongs to another workspace capability.
    CapabilityMismatch,
    /// A filesystem operation failed without a safe path-specific diagnostic.
    FilesystemFailure,
}

impl WorkspacePathErrorCode {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::WorkspaceNotFound => "workspace root does not exist",
            Self::WorkspaceNotDirectory => "workspace root is not a directory",
            Self::AbsolutePath => "absolute workspace paths are not allowed",
            Self::ParentTraversal => "parent path traversal is not allowed",
            Self::InvalidPath => "workspace path is invalid",
            Self::PathTooLong => "workspace path exceeds the byte limit",
            Self::TooManyComponents => "workspace path has too many components",
            Self::TargetNotFound => "workspace target does not exist",
            Self::WorkspaceRootMutation => "the workspace root cannot be mutated as a target",
            Self::OutsideWorkspace => "workspace path resolves outside the workspace",
            Self::ParentNotDirectory => "the nearest existing parent is not a directory",
            Self::UnresolvableTarget => "workspace target cannot be resolved safely",
            Self::PathChanged => "workspace target changed after resolution",
            Self::CapabilityMismatch => "workspace target belongs to another capability",
            Self::FilesystemFailure => "workspace filesystem operation failed",
        }
    }
}

/// Bounded path-independent error safe for model, terminal, and JSON output.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct WorkspacePathError {
    code: WorkspacePathErrorCode,
    message: &'static str,
}

impl WorkspacePathError {
    pub(crate) const fn new(code: WorkspacePathErrorCode) -> Self {
        let message = code.message();
        debug_assert!(message.len() <= MAX_WORKSPACE_ERROR_MESSAGE_BYTES);
        Self { code, message }
    }

    /// Returns the stable machine-readable failure classification.
    #[must_use]
    pub const fn code(&self) -> WorkspacePathErrorCode {
        self.code
    }

    /// Returns a bounded diagnostic that never embeds caller or host paths.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}
