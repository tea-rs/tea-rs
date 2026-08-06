use std::future::Future;
use std::pin::Pin;

use tea_protocol::CanonicalMessage;

use crate::KernelError;

/// Boxed future returned by a compaction summarizer.
pub type CompactionSummaryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CanonicalMessage, KernelError>> + Send + 'a>>;

/// Policy deciding whether a context overflow should trigger automatic
/// compaction before the next model request.
///
/// The kernel never generates a summary itself; when this policy allows
/// compaction, it requests one from a product-supplied [`CompactionSummarizer`].
pub trait CompactionPolicy: std::fmt::Debug + Send + Sync {
    /// Returns whether automatic compaction should run for an overflow.
    fn should_compact(&self, estimated_input_tokens: usize, context_window: u64) -> bool;
}

/// Default policy that never compacts automatically.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeverCompactPolicy;

impl CompactionPolicy for NeverCompactPolicy {
    fn should_compact(&self, _estimated_input_tokens: usize, _context_window: u64) -> bool {
        false
    }
}

/// Product-supplied source of compaction summaries.
///
/// The kernel never owns a model for compaction. A product wires a summarizer
/// that may call a model, retrieve a cached summary, or produce a deterministic
/// truncation; the testkit provides a deterministic implementation.
pub trait CompactionSummarizer: std::fmt::Debug + Send + Sync {
    /// Produces one assistant summary message for the supplied transcript.
    fn summarize(&self, messages: Vec<CanonicalMessage>) -> CompactionSummaryFuture<'_>;
}
