use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use tea_context::{SkillId, SkillMetadata};

use crate::resources::frontmatter::{field, parse};
use crate::{CodingError, CodingErrorCode};

const MAX_SKILLS: usize = 128;
const MAX_SKILL_CONTENT_BYTES: usize = 128 * 1024;

/// One validated declarative skill whose body is loaded only on invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSkill {
    metadata: SkillMetadata,
    path: PathBuf,
}

impl DiscoveredSkill {
    /// Returns prompt-safe skill metadata.
    #[must_use]
    pub const fn metadata(&self) -> &SkillMetadata {
        &self.metadata
    }

    /// Loads the body after explicit `/skill:name [args]` invocation.
    ///
    /// # Errors
    ///
    /// Rejects invalid syntax, changed metadata, oversized content, and unsafe referenced paths.
    pub fn invoke(&self, invocation: &str) -> Result<LoadedSkill, CodingError> {
        let prefix = format!("/skill:{}", self.metadata.id());
        let args = invocation
            .strip_prefix(&prefix)
            .filter(|remaining| remaining.is_empty() || remaining.starts_with(' '))
            .ok_or_else(invalid)?
            .trim_start()
            .to_owned();
        if args.len() > 16 * 1024 || args.contains('\0') {
            return Err(invalid());
        }
        let source = fs::read_to_string(&self.path).map_err(|_| not_found())?;
        let document = parse(&source)?;
        if field(&document, "name")? != self.metadata.id().as_str() {
            return Err(invalid());
        }
        Ok(LoadedSkill {
            content: document.body,
            arguments: args,
            directory: self.path.parent().ok_or_else(invalid)?.to_path_buf(),
        })
    }
}

/// Explicitly loaded skill body and its safe reference base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSkill {
    content: String,
    arguments: String,
    directory: PathBuf,
}

impl LoadedSkill {
    /// Returns skill instructions.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
    /// Returns uninterpreted invocation arguments.
    #[must_use]
    pub fn arguments(&self) -> &str {
        &self.arguments
    }
    /// Resolves one relative existing reference beneath the skill directory.
    ///
    /// # Errors
    ///
    /// Rejects absolute/traversing/missing references and symlink escape.
    pub fn resolve_reference(&self, relative: &str) -> Result<PathBuf, CodingError> {
        let path = Path::new(relative);
        if relative.is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(invalid());
        }
        let directory = fs::canonicalize(&self.directory).map_err(|_| not_found())?;
        let target = fs::canonicalize(directory.join(path)).map_err(|_| not_found())?;
        if !target.starts_with(&directory) {
            return Err(invalid());
        }
        Ok(target)
    }
}

pub(crate) fn discover(roots: &[PathBuf]) -> Result<Vec<DiscoveredSkill>, CodingError> {
    let mut candidates = Vec::new();
    for root in roots {
        let canonical_root = match fs::canonicalize(root) {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(not_found()),
        };
        let entries = match fs::read_dir(&canonical_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(not_found()),
        };
        let mut directories = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        directories.sort();
        for directory in directories {
            let path = directory.join("SKILL.md");
            match fs::canonicalize(path) {
                Ok(path) if path.starts_with(&canonical_root) && path.is_file() => {
                    candidates.push(path);
                }
                Ok(_) => return Err(invalid()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(not_found()),
            }
        }
    }
    if candidates.len() > MAX_SKILLS {
        return Err(invalid());
    }
    let mut skills = BTreeMap::new();
    for path in candidates {
        let source = fs::read_to_string(&path).map_err(|_| not_found())?;
        if source.len() > MAX_SKILL_CONTENT_BYTES {
            return Err(invalid());
        }
        let document = parse(&source)?;
        let name = field(&document, "name")?;
        let description = field(&document, "description")?;
        let metadata =
            SkillMetadata::new(SkillId::from_str(name).map_err(|_| invalid())?, description)
                .map_err(|_| invalid())?;
        if skills
            .insert(name.to_owned(), DiscoveredSkill { metadata, path })
            .is_some()
        {
            return Err(CodingError::new(
                CodingErrorCode::InvalidInput,
                "duplicate skill name",
            ));
        }
    }
    Ok(skills.into_values().collect())
}

fn invalid() -> CodingError {
    CodingError::new(CodingErrorCode::InvalidInput, "skill resource is invalid")
}
fn not_found() -> CodingError {
    CodingError::new(CodingErrorCode::NotFound, "skill resource is missing")
}
