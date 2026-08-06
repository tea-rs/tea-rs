use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{FileToolError, FileToolErrorCode};

/// Explicit host-owned shell configuration; model arguments cannot replace it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashShell {
    executable: PathBuf,
    command_argument: String,
}

impl BashShell {
    /// Validates an absolute existing shell executable and its command flag.
    ///
    /// # Errors
    ///
    /// Rejects relative/missing/non-file executables and invalid command flags.
    pub fn new(
        executable: impl AsRef<Path>,
        command_argument: impl Into<String>,
    ) -> Result<Self, FileToolError> {
        let executable = executable.as_ref();
        let command_argument = command_argument.into();
        if !executable.is_absolute()
            || command_argument.is_empty()
            || command_argument.len() > 32
            || command_argument.chars().any(char::is_control)
        {
            return Err(FileToolError::new(FileToolErrorCode::InvalidArguments));
        }
        let executable = fs::canonicalize(executable)
            .map_err(|_| FileToolError::new(FileToolErrorCode::NotFound))?;
        if !executable.is_file() {
            return Err(FileToolError::new(FileToolErrorCode::NotAFile));
        }
        Ok(Self {
            executable,
            command_argument,
        })
    }

    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }

    pub(crate) fn command_argument(&self) -> &str {
        &self.command_argument
    }
}

/// Validated protected directory for oversized command output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashOutputDirectory {
    path: PathBuf,
}

impl BashOutputDirectory {
    /// Validates an existing host-owned state directory.
    ///
    /// # Errors
    ///
    /// Rejects missing paths and non-directories.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, FileToolError> {
        let path =
            fs::canonicalize(path).map_err(|_| FileToolError::new(FileToolErrorCode::NotFound))?;
        if !path.is_dir() {
            return Err(FileToolError::new(FileToolErrorCode::NotAFile));
        }
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Bounded process execution configuration owned by the host.
#[derive(Debug, Clone)]
pub struct BashConfig {
    shell: BashShell,
    output_directory: BashOutputDirectory,
    timeout: Duration,
}

impl BashConfig {
    /// Creates process configuration with a non-zero timeout up to 120 seconds.
    ///
    /// # Errors
    ///
    /// Rejects zero or greater-than-24-hour timeouts.
    pub fn new(
        shell: BashShell,
        output_directory: BashOutputDirectory,
        timeout: Duration,
    ) -> Result<Self, FileToolError> {
        if timeout.is_zero() || timeout > Duration::from_mins(2) {
            return Err(FileToolError::new(FileToolErrorCode::InvalidArguments));
        }
        Ok(Self {
            shell,
            output_directory,
            timeout,
        })
    }

    pub(crate) const fn shell(&self) -> &BashShell {
        &self.shell
    }

    pub(crate) const fn output_directory(&self) -> &BashOutputDirectory {
        &self.output_directory
    }

    pub(crate) const fn timeout(&self) -> Duration {
        self.timeout
    }
}
