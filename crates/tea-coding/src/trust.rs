use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CodingError, CodingErrorCode};

const TRUST_SCHEMA_VERSION: u32 = 1;
const MAX_TRUST_FILE_BYTES: usize = 256 * 1024;

/// Persisted project trust decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedTrustDecision {
    /// Project-local settings and resources may be loaded.
    Trusted,
    /// Project-local settings and resources are explicitly rejected.
    Rejected,
    /// Project-local settings and resources are silently ignored.
    Ignored,
}

/// One invocation's effective trust request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustRequest {
    /// Use persisted policy; interactive caller may prompt if undecided.
    Default,
    /// Trust only for this process invocation.
    TrustOnce,
    /// Trust and persist the decision.
    TrustPersisted,
    /// Reject and optionally persist through [`ProjectTrustStore::set`].
    Reject,
    /// Ignore project-local input.
    Ignore,
}

/// Whether the current mode can display a trust prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    /// A user prompt can be shown.
    Interactive,
    /// No prompt can be shown; undecided defaults fail closed.
    NonInteractive,
}

/// Effective project-local resource access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectAccess {
    /// Project-local configuration/resources may be loaded.
    Trusted,
    /// Project-local configuration/resources must be ignored.
    Ignored,
    /// Interactive caller must ask before loading project-local data.
    Ask,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustFile {
    schema_version: u32,
    decisions: BTreeMap<String, PersistedTrustDecision>,
}

/// Atomic persistent project-trust repository keyed by canonical workspace.
#[derive(Debug, Clone)]
pub struct ProjectTrustStore {
    path: PathBuf,
}

impl ProjectTrustStore {
    /// Creates a repository at an injected path without touching a real home.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns a saved decision for the canonical workspace.
    ///
    /// # Errors
    ///
    /// Rejects non-canonical workspaces and invalid/oversized trust files.
    pub fn get(&self, workspace: &Path) -> Result<Option<PersistedTrustDecision>, CodingError> {
        let workspace = canonical_workspace(workspace)?;
        Ok(self.load()?.decisions.get(&workspace).copied())
    }

    /// Atomically persists one decision with owner-only Unix permissions.
    ///
    /// # Errors
    ///
    /// Returns a bounded persistence error if the state cannot be committed.
    pub fn set(
        &self,
        workspace: &Path,
        decision: PersistedTrustDecision,
    ) -> Result<(), CodingError> {
        let workspace = canonical_workspace(workspace)?;
        let mut file = self.load()?;
        file.decisions.insert(workspace, decision);
        let bytes = serde_json::to_vec_pretty(&file).map_err(|_| persistence())?;
        if bytes.len() > MAX_TRUST_FILE_BYTES {
            return Err(persistence());
        }
        let parent = self.path.parent().ok_or_else(persistence)?;
        fs::create_dir_all(parent).map_err(|_| persistence())?;
        let temporary = self.path.with_extension("json.tmp");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(&temporary).map_err(|_| persistence())?;
        output.write_all(&bytes).map_err(|_| persistence())?;
        output.sync_all().map_err(|_| persistence())?;
        fs::rename(&temporary, &self.path).map_err(|_| persistence())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .map_err(|_| persistence())?;
        }
        Ok(())
    }

    /// Resolves a request without reading project-local content.
    ///
    /// # Errors
    ///
    /// Non-interactive undecided/rejected projects fail closed.
    pub fn resolve(
        &self,
        workspace: &Path,
        request: TrustRequest,
        mode: InteractionMode,
    ) -> Result<ProjectAccess, CodingError> {
        match request {
            TrustRequest::TrustOnce => return Ok(ProjectAccess::Trusted),
            TrustRequest::TrustPersisted => {
                self.set(workspace, PersistedTrustDecision::Trusted)?;
                return Ok(ProjectAccess::Trusted);
            }
            TrustRequest::Reject => return Err(not_trusted()),
            TrustRequest::Ignore => return Ok(ProjectAccess::Ignored),
            TrustRequest::Default => {}
        }
        match self.get(workspace)? {
            Some(PersistedTrustDecision::Trusted) => Ok(ProjectAccess::Trusted),
            Some(PersistedTrustDecision::Ignored) => Ok(ProjectAccess::Ignored),
            None if mode == InteractionMode::Interactive => Ok(ProjectAccess::Ask),
            None | Some(PersistedTrustDecision::Rejected) => Err(not_trusted()),
        }
    }

    fn load(&self) -> Result<TrustFile, CodingError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TrustFile {
                    schema_version: TRUST_SCHEMA_VERSION,
                    decisions: BTreeMap::new(),
                });
            }
            Err(_) => return Err(persistence()),
        };
        if bytes.len() > MAX_TRUST_FILE_BYTES {
            return Err(persistence());
        }
        let file = serde_json::from_slice::<TrustFile>(&bytes).map_err(|_| persistence())?;
        if file.schema_version != TRUST_SCHEMA_VERSION || file.decisions.len() > 4096 {
            return Err(persistence());
        }
        Ok(file)
    }
}

fn canonical_workspace(workspace: &Path) -> Result<String, CodingError> {
    let canonical = fs::canonicalize(workspace).map_err(|_| {
        CodingError::new(
            CodingErrorCode::NotFound,
            "workspace directory does not exist",
        )
    })?;
    if !canonical.is_dir() {
        return Err(CodingError::new(
            CodingErrorCode::InvalidInput,
            "workspace is not a directory",
        ));
    }
    canonical
        .to_str()
        .filter(|value| value.len() <= 4096 && !value.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| {
            CodingError::new(
                CodingErrorCode::InvalidInput,
                "workspace identity is invalid",
            )
        })
}

fn persistence() -> CodingError {
    CodingError::new(CodingErrorCode::Persistence, "project trust state failed")
}

fn not_trusted() -> CodingError {
    CodingError::new(
        CodingErrorCode::ProjectNotTrusted,
        "project-local configuration is not trusted",
    )
}
