use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read as _;
use std::path::Path;
use std::str::FromStr;

use tea_context::{PromptSegmentId, TrustLevel, WorkspaceInstruction};

use crate::resources::ResourceDiagnostic;
use crate::{CodingError, CodingErrorCode, ProjectAccess};

const MAX_CONTEXT_FILES: usize = 32;
const MAX_CONTEXT_TOTAL_BYTES: usize = 256 * 1024;

pub(crate) fn discover(
    boundary: &Path,
    workspace: &Path,
    access: ProjectAccess,
) -> Result<(Vec<WorkspaceInstruction>, Vec<ResourceDiagnostic>), CodingError> {
    if access != ProjectAccess::Trusted {
        return Ok((Vec::new(), Vec::new()));
    }
    let boundary = fs::canonicalize(boundary).map_err(|_| not_found())?;
    let workspace = fs::canonicalize(workspace).map_err(|_| not_found())?;
    if !workspace.starts_with(&boundary) {
        return Err(invalid());
    }
    let relative = workspace.strip_prefix(&boundary).map_err(|_| invalid())?;
    let mut directories = vec![boundary.clone()];
    let mut current = boundary.clone();
    for component in relative.components() {
        current = current.join(component);
        directories.push(current.clone());
    }
    let mut instructions = Vec::new();
    let mut diagnostics = Vec::new();
    let mut total = 0_usize;
    for directory in directories {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = directory.join(name);
            let canonical_path = match fs::canonicalize(&path) {
                Ok(path) if path.starts_with(&boundary) => path,
                Ok(_) => return Err(invalid()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    diagnostics.push(ResourceDiagnostic::new("context_read_failed", name));
                    continue;
                }
            };
            let Ok(bytes) = fs::read(&canonical_path) else {
                diagnostics.push(ResourceDiagnostic::new("context_read_failed", name));
                continue;
            };
            let content = String::from_utf8(bytes).map_err(|_| invalid())?;
            if content.trim().is_empty() {
                continue;
            }
            total = total.saturating_add(content.len());
            if instructions.len() == MAX_CONTEXT_FILES || total > MAX_CONTEXT_TOTAL_BYTES {
                return Err(CodingError::new(
                    CodingErrorCode::InvalidInput,
                    "workspace context files exceed bounds",
                ));
            }
            let locator = path
                .strip_prefix(&boundary)
                .map_err(|_| invalid())?
                .to_string_lossy()
                .replace('\\', "/");
            let id = format!("workspace.context.{}", instructions.len());
            instructions.push(
                WorkspaceInstruction::new(
                    PromptSegmentId::from_str(&id).map_err(|_| invalid())?,
                    content,
                    locator,
                    TrustLevel::Delegated,
                )
                .map_err(|_| invalid())?,
            );
        }
    }
    Ok((instructions, diagnostics))
}

pub(crate) fn add_explicit(
    workspace: &tea_coding_tools::WorkspaceRoot,
    paths: &[String],
    instructions: &mut Vec<WorkspaceInstruction>,
) -> Result<(), CodingError> {
    if paths.len() > MAX_CONTEXT_FILES
        || instructions.len().saturating_add(paths.len()) > MAX_CONTEXT_FILES
    {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut total = 0_usize;
    let mut loaded = Vec::with_capacity(paths.len());
    for path in paths {
        let resolved = workspace.resolve_existing(path).map_err(|_| invalid())?;
        if !seen.insert(resolved.display_path().to_owned()) {
            return Err(invalid());
        }
        let mut file = File::open(resolved.host_path()).map_err(|_| invalid())?;
        let metadata = file.metadata().map_err(|_| invalid())?;
        workspace
            .verify_opened_existing(&resolved, &metadata)
            .map_err(|_| invalid())?;
        if !metadata.is_file() {
            return Err(invalid());
        }
        let declared = usize::try_from(metadata.len()).map_err(|_| invalid())?;
        total = total.checked_add(declared).ok_or_else(invalid)?;
        if total > MAX_CONTEXT_TOTAL_BYTES {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(declared);
        file.by_ref()
            .take(u64::try_from(MAX_CONTEXT_TOTAL_BYTES + 1).map_err(|_| invalid())?)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() != declared {
            return Err(invalid());
        }
        workspace
            .revalidate_existing(&resolved)
            .map_err(|_| invalid())?;
        let content = String::from_utf8(bytes).map_err(|_| invalid())?;
        let id = format!(
            "workspace.explicit_context.{}",
            instructions.len() + loaded.len()
        );
        loaded.push(
            WorkspaceInstruction::new(
                PromptSegmentId::from_str(&id).map_err(|_| invalid())?,
                content,
                resolved.display_path(),
                TrustLevel::Delegated,
            )
            .map_err(|_| invalid())?,
        );
    }
    instructions.extend(loaded);
    Ok(())
}

fn invalid() -> CodingError {
    CodingError::new(
        CodingErrorCode::InvalidInput,
        "workspace context is invalid",
    )
}

fn not_found() -> CodingError {
    CodingError::new(
        CodingErrorCode::NotFound,
        "workspace context boundary is missing",
    )
}
