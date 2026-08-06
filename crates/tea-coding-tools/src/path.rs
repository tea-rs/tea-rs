use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::{WorkspacePathError, WorkspacePathErrorCode};

/// Maximum UTF-8 bytes in a model-supplied workspace path.
pub const MAX_WORKSPACE_PATH_BYTES: usize = 4096;
/// Maximum components in a model-supplied workspace path.
pub const MAX_WORKSPACE_PATH_COMPONENTS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedRelativePath {
    display: String,
    components: Vec<OsString>,
}

impl ValidatedRelativePath {
    pub(crate) fn parse(input: &str) -> Result<Self, WorkspacePathError> {
        if input.is_empty()
            || input.as_bytes().contains(&0)
            || input.chars().any(char::is_control)
            || looks_like_windows_prefix(input)
        {
            return Err(WorkspacePathError::new(WorkspacePathErrorCode::InvalidPath));
        }
        if input.len() > MAX_WORKSPACE_PATH_BYTES {
            return Err(WorkspacePathError::new(WorkspacePathErrorCode::PathTooLong));
        }

        let path = Path::new(input);
        if path.is_absolute() {
            return Err(WorkspacePathError::new(
                WorkspacePathErrorCode::AbsolutePath,
            ));
        }

        let mut display_components = Vec::new();
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(value) => {
                    if components.len() == MAX_WORKSPACE_PATH_COMPONENTS {
                        return Err(WorkspacePathError::new(
                            WorkspacePathErrorCode::TooManyComponents,
                        ));
                    }
                    let Some(display) = value.to_str() else {
                        return Err(WorkspacePathError::new(WorkspacePathErrorCode::InvalidPath));
                    };
                    if display.contains(['/', '\\']) {
                        return Err(WorkspacePathError::new(WorkspacePathErrorCode::InvalidPath));
                    }
                    display_components.push(display.to_owned());
                    components.push(value.to_owned());
                }
                Component::ParentDir => {
                    return Err(WorkspacePathError::new(
                        WorkspacePathErrorCode::ParentTraversal,
                    ));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(WorkspacePathError::new(
                        WorkspacePathErrorCode::AbsolutePath,
                    ));
                }
            }
        }

        let display = if display_components.is_empty() {
            ".".to_owned()
        } else {
            display_components.join("/")
        };
        Ok(Self {
            display,
            components,
        })
    }

    pub(crate) fn display(&self) -> &str {
        &self.display
    }

    pub(crate) fn join_to(&self, root: &Path) -> PathBuf {
        self.components
            .iter()
            .fold(root.to_path_buf(), |path, component| path.join(component))
    }
}

fn looks_like_windows_prefix(input: &str) -> bool {
    let bytes = input.as_bytes();
    let drive_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    drive_prefix || input.starts_with("\\\\") || input.contains('\\')
}
