use serde::{Deserialize, Serialize};

use crate::{ProfileError, ProfileErrorCode};

/// Maximum prompt output bytes allowed by any profile.
pub const MAX_PROFILE_PROMPT_BYTES: usize = 1024 * 1024;
/// Maximum conservative estimated tokens allowed by any profile.
pub const MAX_PROFILE_PROMPT_TOKENS: usize = 4_000_000;

/// Declarative prompt budget carried by a product profile.
///
/// The runtime converts this to the context crate's `PromptBudget` at binding
/// time. Both bounds are non-zero and bounded so a profile cannot request an
/// empty or unbounded prompt window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePromptBudget {
    max_bytes: usize,
    max_estimated_tokens: usize,
}

impl ProfilePromptBudget {
    /// Creates validated non-zero prompt budget bounds.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or unsupported values.
    pub fn new(max_bytes: usize, max_estimated_tokens: usize) -> Result<Self, ProfileError> {
        if max_bytes == 0
            || max_bytes > MAX_PROFILE_PROMPT_BYTES
            || max_estimated_tokens == 0
            || max_estimated_tokens > MAX_PROFILE_PROMPT_TOKENS
        {
            return Err(ProfileError::new(
                ProfileErrorCode::UnsupportedValue,
                "profile prompt budget is invalid",
            ));
        }
        Ok(Self {
            max_bytes,
            max_estimated_tokens,
        })
    }

    /// Returns maximum exact output bytes.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
    /// Returns maximum conservative estimated tokens.
    #[must_use]
    pub const fn max_estimated_tokens(self) -> usize {
        self.max_estimated_tokens
    }
}

impl Default for ProfilePromptBudget {
    fn default() -> Self {
        Self {
            max_bytes: 32_768,
            max_estimated_tokens: 8_192,
        }
    }
}
