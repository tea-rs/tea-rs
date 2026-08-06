//! Trusted bounded declarative context, skill, and prompt-template discovery.

mod context_files;
mod frontmatter;
mod prompts;
mod skills;

use std::path::{Path, PathBuf};

use tea_context::{SkillMetadata, WorkspaceInstruction};

pub use prompts::PromptTemplate;
pub use skills::{DiscoveredSkill, LoadedSkill};

use crate::{CodingError, ProjectAccess};

/// Bounded safe diagnostic produced while optional resources are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiagnostic {
    code: &'static str,
    subject: String,
}

impl ResourceDiagnostic {
    pub(crate) fn new(code: &'static str, subject: &str) -> Self {
        Self {
            code,
            subject: subject.to_owned(),
        }
    }
    /// Returns the machine-readable diagnostic code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
    /// Returns a bounded workspace-relative subject.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }
}

/// Immutable deterministic catalog discovered for one workspace.
#[derive(Debug, Clone)]
pub struct ResourceCatalog {
    context: Vec<WorkspaceInstruction>,
    skills: Vec<DiscoveredSkill>,
    prompts: Vec<PromptTemplate>,
    diagnostics: Vec<ResourceDiagnostic>,
}

impl ResourceCatalog {
    /// Discovers global resources and project resources only when trusted.
    ///
    /// # Errors
    ///
    /// Rejects malformed, duplicate, oversized, or escaping resources.
    #[allow(clippy::too_many_arguments)]
    pub fn discover(
        boundary: &Path,
        workspace: &Path,
        access: ProjectAccess,
        global_skill_roots: &[PathBuf],
        project_skill_roots: &[PathBuf],
        global_prompt_root: Option<&Path>,
        project_prompt_root: Option<&Path>,
    ) -> Result<Self, CodingError> {
        let (context, diagnostics) = context_files::discover(boundary, workspace, access)?;
        let mut skill_roots = global_skill_roots.to_vec();
        if access == ProjectAccess::Trusted {
            skill_roots.extend_from_slice(project_skill_roots);
        }
        let skills = skills::discover(&skill_roots)?;
        let prompts = prompts::discover(
            global_prompt_root,
            (access == ProjectAccess::Trusted)
                .then_some(project_prompt_root)
                .flatten(),
        )?;
        Ok(Self {
            context,
            skills,
            prompts,
            diagnostics,
        })
    }

    /// Adds explicit workspace-relative context files through the workspace capability.
    ///
    /// # Errors
    ///
    /// Rejects duplicate, escaping, non-UTF-8, oversized, changed, or unreadable files.
    pub fn add_explicit_context_files(
        &mut self,
        workspace: &tea_coding_tools::WorkspaceRoot,
        paths: &[String],
    ) -> Result<(), CodingError> {
        context_files::add_explicit(workspace, paths, &mut self.context)
    }

    /// Applies resolved resource feature switches without re-reading any source.
    pub fn apply_settings(&mut self, context_files: bool, prompt_templates: bool) {
        if !context_files {
            self.context.clear();
        }
        if !prompt_templates {
            self.prompts.clear();
        }
    }

    /// Returns deterministic context instructions.
    #[must_use]
    pub fn context(&self) -> &[WorkspaceInstruction] {
        &self.context
    }
    /// Returns discovered skills sorted by ID.
    #[must_use]
    pub fn skills(&self) -> &[DiscoveredSkill] {
        &self.skills
    }
    /// Returns prompt templates sorted by name.
    #[must_use]
    pub fn prompts(&self) -> &[PromptTemplate] {
        &self.prompts
    }
    /// Returns safe discovery diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[ResourceDiagnostic] {
        &self.diagnostics
    }
    /// Projects skill metadata without loading skill bodies.
    #[must_use]
    pub fn skill_metadata(&self) -> Vec<SkillMetadata> {
        self.skills
            .iter()
            .map(|skill| skill.metadata().clone())
            .collect()
    }
    /// Loads an explicitly invoked skill.
    ///
    /// # Errors
    ///
    /// Rejects unknown or invalid invocation syntax.
    pub fn invoke_skill(&self, invocation: &str) -> Result<LoadedSkill, CodingError> {
        let name = invocation
            .strip_prefix("/skill:")
            .map(|remaining| {
                remaining
                    .split_once(' ')
                    .map_or(remaining, |(name, _)| name)
            })
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                crate::CodingError::new(
                    crate::CodingErrorCode::InvalidInput,
                    "skill invocation is invalid",
                )
            })?;
        self.skills
            .iter()
            .find(|skill| skill.metadata().id().as_str() == name)
            .ok_or_else(|| {
                crate::CodingError::new(crate::CodingErrorCode::NotFound, "skill is not registered")
            })?
            .invoke(invocation)
    }
}
