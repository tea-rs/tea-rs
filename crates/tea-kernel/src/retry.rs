use std::time::Duration;

use tea_model::{ModelFailure, ModelFailureCode};
use tea_protocol::RetryClass;

use crate::{KernelError, KernelErrorCode};

/// Maximum attempts a kernel run will retry a model request.
pub const MAX_MODEL_RETRY_ATTEMPTS: u32 = 8;
/// Maximum base delay for a model retry policy.
pub const MAX_MODEL_RETRY_BASE_DELAY: Duration = Duration::from_secs(30);
/// Maximum cap on model retry delays.
pub const MAX_MODEL_RETRY_MAX_DELAY: Duration = Duration::from_mins(5);

/// Deterministic provider-neutral model retry policy.
///
/// The policy is jitter-free for test determinism: `next_delay` is a pure
/// exponential backoff `base * 2^(attempt-1)` capped at `max_delay`. Retries
/// are bounded by `max_attempts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelRetryPolicy {
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl ModelRetryPolicy {
    /// Creates a validated retry policy.
    ///
    /// # Errors
    ///
    /// Returns an error for zero attempts, zero base delay, a base greater than
    /// max, or values above the supported bounds.
    pub fn new(
        max_attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
    ) -> Result<Self, KernelError> {
        if max_attempts == 0
            || max_attempts > MAX_MODEL_RETRY_ATTEMPTS
            || base_delay.is_zero()
            || base_delay > MAX_MODEL_RETRY_BASE_DELAY
            || max_delay < base_delay
            || max_delay > MAX_MODEL_RETRY_MAX_DELAY
        {
            return Err(KernelError::new(
                KernelErrorCode::InvalidRequest,
                "model retry policy is invalid",
            ));
        }
        Ok(Self {
            max_attempts,
            base_delay,
            max_delay,
        })
    }

    /// Returns the maximum attempts including the first try.
    #[must_use]
    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }
    /// Returns the base backoff delay applied to the first retry.
    #[must_use]
    pub const fn base_delay(self) -> Duration {
        self.base_delay
    }
    /// Returns the cap applied to every computed delay.
    #[must_use]
    pub const fn max_delay(self) -> Duration {
        self.max_delay
    }

    /// Returns the deterministic delay to wait before the next attempt.
    ///
    /// `attempt` is 1-indexed: the first retry (attempt 1) waits `base_delay`.
    #[must_use]
    pub fn next_delay(self, attempt: u32) -> Duration {
        if attempt == 0 {
            return self.base_delay;
        }
        let exponent = attempt.min(20).saturating_sub(1);
        let scaled = self
            .base_delay
            .checked_mul(2u32.saturating_pow(exponent))
            .unwrap_or(self.max_delay);
        scaled.min(self.max_delay)
    }

    /// Returns the bounded delay selected for one retryable failure.
    ///
    /// A provider-requested delay takes precedence over local exponential
    /// backoff. Both sources are capped by `max_delay`.
    #[must_use]
    pub fn delay_for_failure(self, failure: &ModelFailure, attempt: u32) -> Duration {
        failure
            .retry_after()
            .unwrap_or_else(|| self.next_delay(attempt))
            .min(self.max_delay)
    }

    /// Returns the failure code when a retry is permitted, else `None`.
    ///
    /// `Never` failures (invalid request, auth, permission, malformed, or
    /// cancellation) are never retried. `Immediate` and `AfterBackoff` failures
    /// are retryable within the policy attempt bound; the caller checks the
    /// attempt count separately.
    #[must_use]
    pub fn should_retry(self, failure: &ModelFailure) -> Option<ModelFailureCode> {
        match failure.retry() {
            RetryClass::Never => None,
            RetryClass::Immediate | RetryClass::AfterBackoff => Some(failure.code()),
        }
    }
}

impl Default for ModelRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        }
    }
}
