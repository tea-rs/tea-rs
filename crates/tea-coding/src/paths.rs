use std::path::{Path, PathBuf};

use crate::{CodingError, CodingErrorCode};

/// Fully injected filesystem roots used by the coding product.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)] // `_dir` distinguishes injected directory roots.
pub struct AppPaths {
    config_dir: PathBuf,
    state_dir: PathBuf,
    data_dir: PathBuf,
}

impl AppPaths {
    /// Creates paths without consulting process environment or a real home.
    ///
    /// # Errors
    ///
    /// Rejects non-absolute roots or roots containing no final component.
    pub fn new(
        config_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
    ) -> Result<Self, CodingError> {
        let paths = [config_dir.into(), state_dir.into(), data_dir.into()];
        if paths
            .iter()
            .any(|path| !path.is_absolute() || path.file_name().is_none())
        {
            return Err(CodingError::new(
                CodingErrorCode::InvalidInput,
                "application paths must be absolute directory paths",
            ));
        }
        let [config_dir, state_dir, data_dir] = paths;
        Ok(Self {
            config_dir,
            state_dir,
            data_dir,
        })
    }

    /// Returns the configuration directory.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Returns the durable state directory.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Returns the declarative resource data directory.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Returns the global settings file.
    #[must_use]
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    /// Returns the global provider configuration file.
    #[must_use]
    pub fn providers_file(&self) -> PathBuf {
        self.config_dir.join("providers.json")
    }

    /// Returns the trust decision store.
    #[must_use]
    pub fn trust_file(&self) -> PathBuf {
        self.state_dir.join("project-trust.json")
    }

    /// Returns the `SQLite` session database path.
    #[must_use]
    pub fn session_database(&self) -> PathBuf {
        self.state_dir.join("sessions.sqlite3")
    }
}
