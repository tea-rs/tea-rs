use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{PromptAuthority, PromptModuleId, PromptSegment};

/// Maximum segments contributed by one module.
pub const MAX_MODULE_SEGMENTS: usize = 256;

/// Bounded priority used only within one authority class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PromptPriority(i16);

impl PromptPriority {
    /// Creates a priority from its signed bounded representation.
    #[must_use]
    pub const fn new(value: i16) -> Self {
        Self(value)
    }
    /// Returns numeric priority; greater values sort first within authority.
    #[must_use]
    pub const fn get(self) -> i16 {
        self.0
    }
}

/// Ordered prompt fragments contributed at one authority and priority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptModule {
    id: PromptModuleId,
    authority: PromptAuthority,
    priority: PromptPriority,
    segments: Vec<PromptSegment>,
}

impl PromptModule {
    /// Creates a validated non-empty prompt module.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty/oversized collection or duplicate segment IDs.
    pub fn new(
        id: PromptModuleId,
        authority: PromptAuthority,
        priority: PromptPriority,
        segments: Vec<PromptSegment>,
    ) -> Result<Self, ModuleError> {
        if segments.is_empty() || segments.len() > MAX_MODULE_SEGMENTS {
            return Err(ModuleError::InvalidSegmentCount);
        }
        let mut ids = BTreeSet::new();
        if segments.iter().any(|segment| !ids.insert(segment.id())) {
            return Err(ModuleError::DuplicateSegmentId);
        }
        Ok(Self {
            id,
            authority,
            priority,
            segments,
        })
    }

    /// Returns module identity.
    #[must_use]
    pub const fn id(&self) -> &PromptModuleId {
        &self.id
    }
    /// Returns fixed authority.
    #[must_use]
    pub const fn authority(&self) -> PromptAuthority {
        self.authority
    }
    /// Returns within-authority priority.
    #[must_use]
    pub const fn priority(&self) -> PromptPriority {
        self.priority
    }
    /// Returns source-ordered segments.
    #[must_use]
    pub fn segments(&self) -> &[PromptSegment] {
        &self.segments
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPromptModule {
    id: PromptModuleId,
    authority: PromptAuthority,
    priority: PromptPriority,
    segments: Vec<PromptSegment>,
}

impl<'de> Deserialize<'de> for PromptModule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPromptModule::deserialize(deserializer)?;
        Self::new(raw.id, raw.authority, raw.priority, raw.segments)
            .map_err(serde::de::Error::custom)
    }
}

/// Prompt module invariant failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ModuleError {
    /// Module contains no segments or too many segments.
    #[error("prompt module segment count is invalid")]
    InvalidSegmentCount,
    /// One segment identity appears more than once in a module.
    #[error("prompt module contains a duplicate segment ID")]
    DuplicateSegmentId,
}
