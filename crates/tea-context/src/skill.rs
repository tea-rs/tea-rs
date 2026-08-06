use std::fmt;
use std::str::FromStr;

use thiserror::Error;

use crate::SkillId;

/// Maximum UTF-8 bytes in one skill description.
pub const MAX_SKILL_DESCRIPTION_BYTES: usize = 4096;

/// Bounded declarative skill metadata; it does not execute the skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadata {
    id: SkillId,
    description: String,
}

impl SkillMetadata {
    /// Creates one skill metadata entry.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or null-containing description.
    pub fn new(id: SkillId, description: impl Into<String>) -> Result<Self, SkillError> {
        let description = description.into();
        if description.is_empty()
            || description.len() > MAX_SKILL_DESCRIPTION_BYTES
            || description.contains('\0')
        {
            return Err(SkillError::InvalidDescription);
        }
        Ok(Self { id, description })
    }
    /// Returns skill identity.
    #[must_use]
    pub const fn id(&self) -> &SkillId {
        &self.id
    }
    /// Returns model-visible skill description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Returns the sole explicit invocation form.
    #[must_use]
    pub fn invocation(&self) -> SkillInvocation {
        SkillInvocation {
            skill_id: self.id.clone(),
        }
    }
}

/// Parsed explicit `@skill <skill-id>` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInvocation {
    skill_id: SkillId,
}

impl SkillInvocation {
    /// Returns invoked skill.
    #[must_use]
    pub const fn skill_id(&self) -> &SkillId {
        &self.skill_id
    }
}

impl FromStr for SkillInvocation {
    type Err = SkillError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let skill_id = value
            .strip_prefix("@skill ")
            .filter(|remaining| !remaining.contains(' '))
            .ok_or(SkillError::InvalidInvocation)?
            .parse()
            .map_err(|_| SkillError::InvalidInvocation)?;
        Ok(Self { skill_id })
    }
}

impl fmt::Display for SkillInvocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "@skill {}", self.skill_id)
    }
}

/// Invalid skill metadata or invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SkillError {
    /// Description violates bounds.
    #[error("skill description is invalid")]
    InvalidDescription,
    /// Invocation is not exact explicit skill syntax.
    #[error("skill invocation must use exact '@skill <skill-id>' syntax")]
    InvalidInvocation,
}
