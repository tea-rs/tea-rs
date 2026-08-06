use std::fmt::Debug;
use std::pin::Pin;

use futures_core::Stream;
use tea_protocol::ModelId;

use crate::{ModelCancellation, ModelEvent, ModelRequest, ModelSpec, ProviderId};

/// Provider-neutral asynchronous stream of normalized model events.
///
/// Streams should be lazy: polling owns request progress. Implementations must
/// not create detached tasks. A fully consumed stream starts with exactly one
/// `Started` event and ends with exactly one `Completed` or `Failed` event.
pub trait ModelStream: Stream<Item = ModelEvent> + Send {}

impl<T> ModelStream for T where T: Stream<Item = ModelEvent> + Send {}

/// Heap-owned object-safe model stream returned by provider adapters.
pub type BoxModelStream = Pin<Box<dyn ModelStream + 'static>>;

/// Object-safe provider-neutral model adapter port.
///
/// Request setup, transport, protocol, cancellation, and runtime failures must
/// be emitted through the returned stream as terminal failure events. Provider
/// implementations must not panic for expected failures or create a nested
/// asynchronous runtime.
pub trait ModelProvider: Debug + Send + Sync {
    /// Returns the stable adapter/provider identity.
    fn provider_id(&self) -> &ProviderId;

    /// Returns models advertised by this adapter in deterministic order.
    fn models(&self) -> &[ModelSpec];

    /// Finds an advertised model by canonical ID.
    fn model(&self, model_id: &ModelId) -> Option<&ModelSpec> {
        self.models()
            .iter()
            .find(|model| model.model_id() == model_id)
    }

    /// Creates a lazy normalized stream for one immutable request.
    ///
    /// `cancellation` is cooperative. Completion must not be reported until
    /// resources owned directly by the stream have been cleaned up. Dropping
    /// the stream abandons it; implementations must therefore keep resource
    /// ownership inside the stream rather than a detached task.
    fn stream(&self, request: ModelRequest, cancellation: ModelCancellation) -> BoxModelStream;
}
