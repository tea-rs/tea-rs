use std::ops::Range;

use crate::{CacheScope, PromptModuleId, PromptProvenance, PromptSegmentId, TrustLevel};

/// Final compiler disposition of one input segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentDisposition {
    /// Full content appears in output.
    Included,
    /// Truncated content with marker appears in output.
    Truncated,
    /// Exact duplicate was deduplicated.
    Duplicate,
    /// Lower-precedence conflict was shadowed.
    ConflictShadowed,
    /// Explicit omit/truncate behavior could not fit the budget.
    OmittedForBudget,
}

/// Explainable inspection row for one input prompt segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptInspectionEntry {
    module_id: PromptModuleId,
    segment_id: PromptSegmentId,
    provenance: PromptProvenance,
    trust: TrustLevel,
    cache_scope: CacheScope,
    disposition: SegmentDisposition,
    byte_range: Option<Range<usize>>,
    rendered_bytes: usize,
    estimated_tokens: usize,
}

impl PromptInspectionEntry {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        module_id: PromptModuleId,
        segment_id: PromptSegmentId,
        provenance: PromptProvenance,
        trust: TrustLevel,
        cache_scope: CacheScope,
        disposition: SegmentDisposition,
        byte_range: Option<Range<usize>>,
        rendered_bytes: usize,
        estimated_tokens: usize,
    ) -> Self {
        Self {
            module_id,
            segment_id,
            provenance,
            trust,
            cache_scope,
            disposition,
            byte_range,
            rendered_bytes,
            estimated_tokens,
        }
    }
    /// Returns source module.
    #[must_use]
    pub const fn module_id(&self) -> &PromptModuleId {
        &self.module_id
    }
    /// Returns source segment.
    #[must_use]
    pub const fn segment_id(&self) -> &PromptSegmentId {
        &self.segment_id
    }
    /// Returns source provenance.
    #[must_use]
    pub const fn provenance(&self) -> &PromptProvenance {
        &self.provenance
    }
    /// Returns trust label.
    #[must_use]
    pub const fn trust(&self) -> TrustLevel {
        self.trust
    }
    /// Returns cache scope.
    #[must_use]
    pub const fn cache_scope(&self) -> CacheScope {
        self.cache_scope
    }
    /// Returns final disposition.
    #[must_use]
    pub const fn disposition(&self) -> SegmentDisposition {
        self.disposition
    }
    /// Returns exact output content range, excluding separators.
    #[must_use]
    pub const fn byte_range(&self) -> Option<&Range<usize>> {
        self.byte_range.as_ref()
    }
    /// Returns rendered content bytes excluding separators.
    #[must_use]
    pub const fn rendered_bytes(&self) -> usize {
        self.rendered_bytes
    }
    /// Returns conservative rendered-content token estimate.
    #[must_use]
    pub const fn estimated_tokens(&self) -> usize {
        self.estimated_tokens
    }
}
