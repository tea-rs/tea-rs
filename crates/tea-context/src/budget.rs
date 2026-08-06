use crate::{ContextError, ContextErrorCode};

/// Fixed separator between accepted prompt segments.
pub const PROMPT_SEPARATOR: &str = "\n\n";
/// Fixed marker appended to deterministically shortened segments.
pub const TRUNCATION_MARKER: &str = "[truncated]";

/// Exact byte and conservative token limits for one compiled prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptBudget {
    max_bytes: usize,
    max_estimated_tokens: usize,
}

impl PromptBudget {
    /// Creates non-zero bounded prompt limits.
    ///
    /// # Errors
    ///
    /// Returns an error for zero values or limits above 16 MiB / safe integer.
    pub fn new(max_bytes: usize, max_estimated_tokens: usize) -> Result<Self, ContextError> {
        if max_bytes == 0
            || max_bytes > 16 * 1024 * 1024
            || max_estimated_tokens == 0
            || u64::try_from(max_estimated_tokens)
                .map_or(true, |value| value > tea_protocol::MAX_SAFE_INTEGER)
        {
            return Err(ContextError::new(
                ContextErrorCode::InvalidValue,
                "prompt budget is invalid",
            ));
        }
        Ok(Self {
            max_bytes,
            max_estimated_tokens,
        })
    }
    /// Returns exact output byte limit.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
    /// Returns conservative estimated-token limit.
    #[must_use]
    pub const fn max_estimated_tokens(self) -> usize {
        self.max_estimated_tokens
    }
}

/// Deterministic conservative token estimate `ceil(utf8_bytes / 3)`.
#[must_use]
pub const fn estimate_tokens(bytes: usize) -> usize {
    bytes.saturating_add(2) / 3
}

pub(crate) fn effective_remaining_bytes(budget: PromptBudget, used_bytes: usize) -> usize {
    let byte_remaining = budget.max_bytes.saturating_sub(used_bytes);
    let token_capacity = budget
        .max_estimated_tokens
        .saturating_mul(3)
        .saturating_sub(used_bytes);
    byte_remaining.min(token_capacity)
}

pub(crate) fn truncate(content: &str, maximum: usize) -> Option<String> {
    if maximum < TRUNCATION_MARKER.len() {
        return None;
    }
    let content_limit = maximum - TRUNCATION_MARKER.len();
    let mut boundary = content_limit.min(content.len());
    while boundary > 0 && !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let mut output = content[..boundary].to_owned();
    output.push_str(TRUNCATION_MARKER);
    Some(output)
}
