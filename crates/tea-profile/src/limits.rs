use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{ProfileError, ProfileErrorCode};

/// Maximum tool iterations allowed by any profile.
pub const MAX_PROFILE_TOOL_ITERATIONS: u32 = 1_024;
/// Maximum elapsed run time allowed by any profile (24 hours).
pub const MAX_PROFILE_ELAPSED: Duration = Duration::from_hours(24);
/// Maximum accumulated assistant output bytes allowed by any profile.
pub const MAX_PROFILE_ASSISTANT_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
/// Maximum emitted events allowed by any profile.
pub const MAX_PROFILE_EVENTS: u64 = 1_000_000;
/// Maximum queued messages allowed by any profile.
pub const MAX_PROFILE_QUEUED_MESSAGES: usize = 1024;

/// Declarative run limits carried by a product profile.
///
/// The runtime converts these to the kernel's `RunLimits` at binding time.
/// Every bound mirrors the kernel's deterministic limits so a profile can never
/// request a looser limit than the kernel can enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)] // `max_*` expresses the stable limit vocabulary.
pub struct ProfileRunLimits {
    max_tool_iterations: u32,
    max_elapsed_millis: u64,
    max_assistant_output_bytes: usize,
    max_events: u64,
    max_queued_messages: usize,
}

impl ProfileRunLimits {
    /// Creates validated non-zero run limits.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or unsupported values.
    pub fn new(
        max_tool_iterations: u32,
        max_elapsed: Duration,
        max_assistant_output_bytes: usize,
        max_events: u64,
        max_queued_messages: usize,
    ) -> Result<Self, ProfileError> {
        if max_tool_iterations == 0
            || max_tool_iterations > MAX_PROFILE_TOOL_ITERATIONS
            || max_elapsed.is_zero()
            || max_elapsed > MAX_PROFILE_ELAPSED
            || max_assistant_output_bytes == 0
            || max_assistant_output_bytes > MAX_PROFILE_ASSISTANT_OUTPUT_BYTES
            || max_events == 0
            || max_events > MAX_PROFILE_EVENTS
            || max_queued_messages == 0
            || max_queued_messages > MAX_PROFILE_QUEUED_MESSAGES
        {
            return Err(ProfileError::new(
                ProfileErrorCode::UnsupportedValue,
                "profile run limits are invalid",
            ));
        }
        let max_elapsed_millis = max_elapsed.as_millis().try_into().map_err(|_| {
            ProfileError::new(
                ProfileErrorCode::UnsupportedValue,
                "profile elapsed limit is out of range",
            )
        })?;
        Ok(Self {
            max_tool_iterations,
            max_elapsed_millis,
            max_assistant_output_bytes,
            max_events,
            max_queued_messages,
        })
    }

    /// Returns the maximum model responses containing tools.
    #[must_use]
    pub const fn max_tool_iterations(self) -> u32 {
        self.max_tool_iterations
    }
    /// Returns maximum elapsed run time.
    #[must_use]
    pub fn max_elapsed(self) -> Duration {
        Duration::from_millis(self.max_elapsed_millis)
    }
    /// Returns maximum accumulated assistant output bytes.
    #[must_use]
    pub const fn max_assistant_output_bytes(self) -> usize {
        self.max_assistant_output_bytes
    }
    /// Returns maximum emitted observations.
    #[must_use]
    pub const fn max_events(self) -> u64 {
        self.max_events
    }
    /// Returns maximum accepted queued messages.
    #[must_use]
    pub const fn max_queued_messages(self) -> usize {
        self.max_queued_messages
    }
}

impl Default for ProfileRunLimits {
    fn default() -> Self {
        Self {
            max_tool_iterations: 16,
            max_elapsed_millis: 300_000,
            max_assistant_output_bytes: 4 * 1024 * 1024,
            max_events: 100_000,
            max_queued_messages: 64,
        }
    }
}
