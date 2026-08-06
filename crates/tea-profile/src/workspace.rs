use serde::{Deserialize, Serialize};

use crate::identity::{ProfileSegmentId, ProfileTrustLevel};
use crate::{ProfileError, ProfileErrorCode};

/// Maximum UTF-8 bytes in a workspace instruction content document.
pub const MAX_WORKSPACE_CONTENT_BYTES: usize = 256 * 1024;
/// Maximum UTF-8 bytes in a workspace instruction locator.
pub const MAX_WORKSPACE_LOCATOR_BYTES: usize = 2_048;
/// Maximum workspace instructions carried by one profile.
pub const MAX_PROFILE_WORKSPACE_INSTRUCTIONS: usize = 128;

/// One caller-supplied workspace instruction document declared by a profile.
///
/// The runtime converts each instruction to a context `WorkspaceInstruction`
/// at binding time. The profile never reads files; the locator is declarative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileWorkspaceInstruction {
    segment_id: ProfileSegmentId,
    content: String,
    locator: String,
    trust: ProfileTrustLevel,
}

impl ProfileWorkspaceInstruction {
    /// Creates a validated workspace instruction.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, oversized, null-containing, or control-byte
    /// content or locator values.
    pub fn new(
        segment_id: ProfileSegmentId,
        content: impl Into<String>,
        locator: impl Into<String>,
        trust: ProfileTrustLevel,
    ) -> Result<Self, ProfileError> {
        let content = content.into();
        let locator = locator.into();
        if content.is_empty()
            || content.len() > MAX_WORKSPACE_CONTENT_BYTES
            || content.contains('\0')
            || locator.is_empty()
            || locator.len() > MAX_WORKSPACE_LOCATOR_BYTES
            || locator.contains('\0')
            || locator.chars().any(char::is_control)
        {
            return Err(ProfileError::new(
                ProfileErrorCode::InvalidSelector,
                "workspace instruction content or locator is invalid",
            ));
        }
        Ok(Self {
            segment_id,
            content,
            locator,
            trust,
        })
    }

    /// Returns the canonical segment identity.
    #[must_use]
    pub fn segment_id(&self) -> &ProfileSegmentId {
        &self.segment_id
    }
    /// Returns the bounded content document.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
    /// Returns the bounded source locator.
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }
    /// Returns the declared trust label.
    #[must_use]
    pub fn trust(&self) -> ProfileTrustLevel {
        self.trust
    }
}
