#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared operation control primitives for `tea-rs`.
//!
//! # Example
//!
//! ```
//! use tea_control::CancellationScope;
//!
//! let root = CancellationScope::new();
//! let child = root.child();
//! root.cancel();
//! assert!(child.is_cancelled());
//! ```

use tokio_util::sync::CancellationToken;

/// Cooperative cancellation scope for an owned operation and its children.
///
/// The implementation wraps Tokio internally, while the public contract does
/// not expose Tokio tokens, runtime handles, channels, or tasks. Cancellation
/// is idempotent and propagates from parent scopes to children.
#[derive(Debug, Clone)]
pub struct CancellationScope {
    inner: CancellationToken,
}

impl CancellationScope {
    /// Creates a pending root cancellation scope.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: CancellationToken::new(),
        }
    }

    /// Creates a child cancelled when this parent is cancelled.
    ///
    /// Cancelling the child does not cancel this parent.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            inner: self.inner.child_token(),
        }
    }

    /// Requests cooperative cancellation for this scope and its children.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Waits until cancellation is requested.
    pub async fn cancelled(&self) {
        self.inner.cancelled().await;
    }
}

impl Default for CancellationScope {
    fn default() -> Self {
        Self::new()
    }
}
