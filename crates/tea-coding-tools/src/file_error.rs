use thiserror::Error;

use crate::{WorkspacePathError, WorkspacePathErrorCode};

/// Stable machine-readable native file-tool failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileToolErrorCode {
    /// Invocation arguments were absent despite schema validation.
    InvalidArguments,
    /// A workspace path was invalid, changed, or escaped containment.
    InvalidPath,
    /// The requested target was not found.
    NotFound,
    /// A regular file was required.
    NotAFile,
    /// File contents were not supported UTF-8 text.
    InvalidUtf8,
    /// File contents appeared to be binary.
    BinaryFile,
    /// A source or resulting file exceeded deterministic bounds.
    TooLarge,
    /// Exact edit text was not present.
    NoMatch,
    /// Exact edit text had an unexpected number of matches.
    MatchCountMismatch,
    /// The file changed after it was read for editing.
    PathChanged,
    /// A safe atomic commit could not be completed.
    AtomicCommitFailed,
    /// A filesystem operation failed without exposing host diagnostics.
    FilesystemFailure,
    /// A configured process could not be spawned or awaited.
    ProcessFailure,
    /// Captured process output exceeded its spill bound.
    OutputLimit,
    /// An internal result-contract invariant failed.
    Internal,
}

impl FileToolErrorCode {
    /// Returns the canonical snake-case code used in structured details.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::InvalidPath => "invalid_path",
            Self::NotFound => "not_found",
            Self::NotAFile => "not_a_file",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::BinaryFile => "binary_file",
            Self::TooLarge => "too_large",
            Self::NoMatch => "no_match",
            Self::MatchCountMismatch => "match_count_mismatch",
            Self::PathChanged => "path_changed",
            Self::AtomicCommitFailed => "atomic_commit_failed",
            Self::FilesystemFailure => "filesystem_failure",
            Self::ProcessFailure => "process_failure",
            Self::OutputLimit => "output_limit",
            Self::Internal => "internal",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::InvalidArguments => "validated file tool arguments are invalid",
            Self::InvalidPath => "workspace file path is invalid or changed",
            Self::NotFound => "workspace file does not exist",
            Self::NotAFile => "workspace target is not a regular file",
            Self::InvalidUtf8 => "workspace file is not valid UTF-8 text",
            Self::BinaryFile => "workspace file appears to be binary",
            Self::TooLarge => "workspace file exceeds the supported byte limit",
            Self::NoMatch => "edit text was not found",
            Self::MatchCountMismatch => "edit text match count is not the expected value",
            Self::PathChanged => "workspace file changed during the edit operation",
            Self::AtomicCommitFailed => "workspace file could not be replaced atomically",
            Self::FilesystemFailure => "workspace file operation failed",
            Self::ProcessFailure => "configured process execution failed",
            Self::OutputLimit => "process output exceeded the supported spill limit",
            Self::Internal => "tool produced an invalid internal result",
        }
    }
}

/// Bounded path-independent file tool error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct FileToolError {
    code: FileToolErrorCode,
    message: &'static str,
}

impl FileToolError {
    pub(crate) const fn new(code: FileToolErrorCode) -> Self {
        Self {
            code,
            message: code.message(),
        }
    }

    /// Returns the stable machine-readable classification.
    #[must_use]
    pub const fn code(self) -> FileToolErrorCode {
        self.code
    }

    /// Returns a path-independent technical message.
    #[must_use]
    pub const fn message(self) -> &'static str {
        self.message
    }
}

impl From<WorkspacePathError> for FileToolError {
    fn from(error: WorkspacePathError) -> Self {
        let code = match error.code() {
            WorkspacePathErrorCode::TargetNotFound => FileToolErrorCode::NotFound,
            WorkspacePathErrorCode::FilesystemFailure => FileToolErrorCode::FilesystemFailure,
            _ => FileToolErrorCode::InvalidPath,
        };
        Self::new(code)
    }
}
