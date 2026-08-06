use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{CacheScope, ConflictKey, PromptProvenance, PromptSegmentId, TrustLevel};

/// Maximum UTF-8 bytes in one prompt segment.
pub const MAX_SEGMENT_BYTES: usize = 1024 * 1024;

/// Behavior when a selected segment cannot fit the remaining prompt budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetBehavior {
    /// Compilation fails rather than dropping or changing the segment.
    Required,
    /// Content may be deterministically shortened with an explicit marker.
    Truncate,
    /// Segment may be omitted with a diagnostic.
    Omit,
}

/// Whether a selected conflict claim may be replaced by higher precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictMode {
    /// Lower-precedence contenders are explicitly rejected as protected.
    Protected,
    /// A higher-precedence contender may replace this claim with diagnostics.
    Replaceable,
}

/// Typed conflict claim attached to one segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictClaim {
    key: ConflictKey,
    mode: ConflictMode,
}

impl ConflictClaim {
    /// Creates one conflict claim.
    #[must_use]
    pub const fn new(key: ConflictKey, mode: ConflictMode) -> Self {
        Self { key, mode }
    }
    /// Returns the conflict key.
    #[must_use]
    pub const fn key(&self) -> &ConflictKey {
        &self.key
    }
    /// Returns replacement behavior.
    #[must_use]
    pub const fn mode(&self) -> ConflictMode {
        self.mode
    }
}

/// One bounded sourced prompt fragment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSegment {
    id: PromptSegmentId,
    content: String,
    provenance: PromptProvenance,
    trust: TrustLevel,
    cache_scope: CacheScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict: Option<ConflictClaim>,
    budget_behavior: BudgetBehavior,
}

impl PromptSegment {
    /// Creates a validated prompt segment.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, or null-containing content.
    pub fn new(
        id: PromptSegmentId,
        content: impl Into<String>,
        provenance: PromptProvenance,
        trust: TrustLevel,
        cache_scope: CacheScope,
        budget_behavior: BudgetBehavior,
    ) -> Result<Self, SegmentError> {
        let content = content.into();
        validate_content(&content)?;
        Ok(Self {
            id,
            content,
            provenance,
            trust,
            cache_scope,
            conflict: None,
            budget_behavior,
        })
    }

    /// Adds a typed conflict claim.
    #[must_use]
    pub fn with_conflict(mut self, conflict: ConflictClaim) -> Self {
        self.conflict = Some(conflict);
        self
    }

    /// Returns segment identity.
    #[must_use]
    pub const fn id(&self) -> &PromptSegmentId {
        &self.id
    }
    /// Returns exact segment content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
    /// Returns source attribution.
    #[must_use]
    pub const fn provenance(&self) -> &PromptProvenance {
        &self.provenance
    }
    /// Returns source trust label.
    #[must_use]
    pub const fn trust(&self) -> TrustLevel {
        self.trust
    }
    /// Returns intended cache scope.
    #[must_use]
    pub const fn cache_scope(&self) -> CacheScope {
        self.cache_scope
    }
    /// Returns optional conflict claim.
    #[must_use]
    pub const fn conflict(&self) -> Option<&ConflictClaim> {
        self.conflict.as_ref()
    }
    /// Returns overflow behavior.
    #[must_use]
    pub const fn budget_behavior(&self) -> BudgetBehavior {
        self.budget_behavior
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPromptSegment {
    id: PromptSegmentId,
    content: String,
    provenance: PromptProvenance,
    trust: TrustLevel,
    cache_scope: CacheScope,
    #[serde(default)]
    conflict: Option<ConflictClaim>,
    budget_behavior: BudgetBehavior,
}

impl<'de> Deserialize<'de> for PromptSegment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPromptSegment::deserialize(deserializer)?;
        let mut segment = Self::new(
            raw.id,
            raw.content,
            raw.provenance,
            raw.trust,
            raw.cache_scope,
            raw.budget_behavior,
        )
        .map_err(serde::de::Error::custom)?;
        segment.conflict = raw.conflict;
        Ok(segment)
    }
}

/// Invalid prompt segment content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("prompt segment content is invalid")]
pub struct SegmentError;

fn validate_content(content: &str) -> Result<(), SegmentError> {
    if content.is_empty() || content.len() > MAX_SEGMENT_BYTES || content.contains('\0') {
        Err(SegmentError)
    } else {
        Ok(())
    }
}
